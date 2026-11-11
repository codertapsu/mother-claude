//! Files pushed to another computer in chunks (docs/PLAN-MACHINES.md Phase
//! 3a, decision 1), shared by moves (`moves`) and code sync (`sync`): the
//! sender `PUT`s at most [`CHUNK`] bytes at a time with the offset it sends
//! from (`…/files/<name>?offset=N`); the receiver appends a chunk only at
//! the size it has (else answers `409` with that size, so the sender goes
//! on from there) into `<name>.part`, and checks the whole file's size and
//! SHA-256 before it takes it ([`finish`]). A broken transfer resumes where
//! it stopped.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::body::Bytes;
use serde_json::Value;

use crate::state::AppState;

/// The largest chunk a sender sends in one request.
pub const CHUNK: usize = 8 * 1024 * 1024;
/// How long one chunk may take (over a slow relay, 8 MiB is a minute).
pub const CHUNK_TIMEOUT: Duration = Duration::from_secs(180);
/// How many times a chunk is tried before the transfer fails.
const TRIES: u32 = 5;

// ─── the sender ────────────────────────────────────────────────────────────────

/// `n` bytes of `path` from `offset` (blocking).
pub fn read_chunk(path: &Path, offset: u64, n: usize) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Why a chunk did not go.
#[derive(Debug)]
pub enum PutError {
    /// The other computer refused it: its status and answer.
    Refused(u16, Value),
    /// It answered without the size it has.
    Odd,
    /// It could not be reached, after every try.
    Offline(String),
}

/// One chunk to `path` on `machine` (`…?offset=N` in it), tried again a few
/// times while the other computer can't be reached; answers the size the
/// other computer has of the file after it (a `409` names it too).
pub async fn put(
    state: &AppState,
    machine: &str,
    path: &str,
    bytes: Vec<u8>,
    acting: &str,
) -> Result<u64, PutError> {
    let bytes = Bytes::from(bytes);
    let mut headers: Vec<(&str, String)> =
        vec![("content-type", "application/octet-stream".to_string())];
    if !acting.is_empty() {
        headers.push((super::acting::HEADER, acting.to_string()));
    }
    let mut last = String::new();
    for attempt in 0..TRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1 << attempt.min(4))).await;
        }
        match super::transport::request_raw(
            state,
            machine,
            "PUT",
            path,
            &headers,
            bytes.clone(),
            CHUNK_TIMEOUT,
        )
        .await
        {
            Ok((200, _, out)) | Ok((409, _, out)) => {
                let v: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
                return v
                    .get("size")
                    .or_else(|| v.pointer("/error/size"))
                    .and_then(Value::as_u64)
                    .ok_or(PutError::Odd);
            }
            Ok((status, _, out)) => {
                let v: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
                return Err(PutError::Refused(status, v));
            }
            Err(e) => last = e,
        }
    }
    Err(PutError::Offline(last))
}

// ─── the receiver ──────────────────────────────────────────────────────────────

/// A file's name while it is still arriving.
pub fn part_of(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{name}.part"))
}

/// How much of a file this computer has: all of it once taken, else what
/// arrived.
pub fn have(path: &Path) -> u64 {
    std::fs::metadata(path)
        .or_else(|_| std::fs::metadata(part_of(path)))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Why a chunk was not taken.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// This computer has another part of the file: this much.
    Offset(u64),
    /// More than the file's size, as announced.
    TooLong,
    /// More than [`CHUNK`].
    TooLarge,
}

/// Appends `bytes` to the file arriving at `path` when `offset` is what
/// this computer has of it and the file stays within `total` (blocking);
/// answers the size it has after.
pub fn append(path: &Path, offset: u64, bytes: &[u8], total: u64) -> Result<u64, AppendError> {
    use std::io::Write;
    if bytes.len() > CHUNK {
        return Err(AppendError::Refused(Refusal::TooLarge));
    }
    let size = have(path);
    if offset != size || path.exists() {
        return Err(AppendError::Refused(Refusal::Offset(size.min(total))));
    }
    if offset + bytes.len() as u64 > total {
        return Err(AppendError::Refused(Refusal::TooLong));
    }
    let part = part_of(path);
    let written = (|| -> std::io::Result<u64> {
        if let Some(parent) = part.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&part)?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(file.metadata()?.len())
    })();
    written.map_err(AppendError::Io)
}

/// [`append`]'s failure.
#[derive(Debug)]
pub enum AppendError {
    Refused(Refusal),
    Io(std::io::Error),
}

/// The file at `path` whole and as announced: a part that arrived whole is
/// checked and put in its place; one there already is checked again; an
/// empty one that never needed a chunk is made (blocking).
pub fn finish(path: &Path, size: u64, sha256: &str) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !path.exists() && size == 0 && !part_of(path).exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{name}: {e}"))?;
        }
        std::fs::write(path, b"").map_err(|e| format!("{name}: {e}"))?;
    }
    if !path.exists() {
        let part = part_of(path);
        let (got, sha) = crate::claude::transfer::sha256_file(&part)
            .map_err(|_| format!("{name} did not arrive"))?;
        if got != size || sha != sha256 {
            return Err(format!("{name} did not arrive whole"));
        }
        std::fs::rename(&part, path).map_err(|e| format!("{name}: {e}"))?;
    } else {
        let (got, sha) =
            crate::claude::transfer::sha256_file(path).map_err(|e| format!("{name}: {e}"))?;
        if got != size || sha != sha256 {
            return Err(format!("{name} is not what was sent"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(bytes: &[u8]) -> String {
        crate::claude::secrets::sha256_hex(bytes)
    }

    #[test]
    fn chunks_append_at_the_size_held_and_a_whole_file_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("move.bundle");
        let data: Vec<u8> = (0..1000u32).flat_map(|i| i.to_le_bytes()).collect();
        assert_eq!(have(&path), 0);
        assert_eq!(append(&path, 0, &data[..1500], 4000).unwrap(), 1500);
        // A retried chunk, or one from the wrong place: what is held instead.
        match append(&path, 0, &data[..1500], 4000) {
            Err(AppendError::Refused(Refusal::Offset(1500))) => {}
            other => panic!("{other:?}"),
        }
        match append(&path, 1500, &data[1500..], 3000) {
            Err(AppendError::Refused(Refusal::TooLong)) => {}
            other => panic!("{other:?}"),
        }
        assert!(finish(&path, 4000, &sha(&data)).is_err(), "not whole yet");
        assert_eq!(append(&path, 1500, &data[1500..], 4000).unwrap(), 4000);
        assert!(
            finish(&path, 4000, &sha(b"other")).is_err(),
            "not what was sent"
        );
        finish(&path, 4000, &sha(&data)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert!(!part_of(&path).exists());
        // Taken: no more chunks, and checking it again is fine.
        assert!(append(&path, 4000, b"x", 4001).is_err());
        finish(&path, 4000, &sha(&data)).unwrap();
        // An empty file needs no chunk.
        let empty = dir.path().join("empty");
        finish(&empty, 0, &sha(b"")).unwrap();
        assert_eq!(std::fs::read(&empty).unwrap(), b"");
        assert_eq!(read_chunk(&path, 4, 4).unwrap(), data[4..8].to_vec());
    }
}
