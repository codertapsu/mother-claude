//! A run's output (docs/PLAN-MACHINES.md Phase 4, decision 4): every byte
//! its shell writes, stdout and stderr as they arrive, kept whole in a file
//! next to its record for a day (up to [`MAX_STORED`]) and its last bytes
//! in memory, read back in pages that start and end on a character.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::Serialize;

/// The most of one run's output kept on disk; past it only the tail grows.
pub const MAX_STORED: u64 = 256 << 20;
/// The largest page read at once.
pub const MAX_PAGE: usize = 256 * 1024;
/// The output's file, beside the run's record.
pub const FILE: &str = "output.log";

/// The output of a run as it arrives.
#[derive(Debug)]
pub struct Output {
    file: Option<std::fs::File>,
    stored: u64,
    size: u64,
    tail: VecDeque<u8>,
    keep: usize,
    truncated: bool,
}

impl Output {
    /// Output kept in `path` (none: memory only) and its last `keep` bytes.
    pub fn new(path: Option<&Path>, keep: usize) -> Self {
        let file = path.and_then(|p| {
            let mut opts = std::fs::OpenOptions::new();
            opts.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            match opts.open(p) {
                Ok(f) => Some(f),
                Err(e) => {
                    tracing::warn!(path = %p.display(), error = %e, "a run's output file could not be opened");
                    None
                }
            }
        });
        Self {
            file,
            stored: 0,
            size: 0,
            tail: VecDeque::with_capacity(keep.min(1 << 16)),
            keep,
            truncated: false,
        }
    }

    /// Appends what the run wrote.
    pub fn push(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.size += bytes.len() as u64;
        if let Some(f) = self.file.as_mut() {
            let room = MAX_STORED.saturating_sub(self.stored) as usize;
            let n = bytes.len().min(room);
            if n > 0 {
                if f.write_all(&bytes[..n]).is_err() {
                    self.file = None;
                    self.truncated = true;
                } else {
                    self.stored += n as u64;
                }
            }
            if n < bytes.len() {
                self.truncated = true;
            }
        }
        let take = bytes.len().min(self.keep);
        let drop = (self.tail.len() + take).saturating_sub(self.keep);
        self.tail.drain(..drop.min(self.tail.len()));
        self.tail.extend(&bytes[bytes.len() - take..]);
    }

    /// Bytes written so far.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Whether the file holds less than all of it.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// The last `n` bytes as text, from a character's start.
    pub fn tail(&self, n: usize) -> String {
        let n = n.min(self.tail.len());
        let start = self.tail.len() - n;
        let bytes: Vec<u8> = self.tail.range(start..).copied().collect();
        text_from(&bytes, start > 0 || self.size > self.tail.len() as u64)
    }

    /// Flushes the file.
    pub fn flush(&mut self) {
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }
}

/// `bytes` as text: without the first bytes of a character cut off at the
/// start (`cut`), and without a character cut off at the end.
pub fn text_from(bytes: &[u8], cut: bool) -> String {
    let mut start = 0;
    if cut {
        while start < bytes.len() && start < 3 && (bytes[start] & 0xC0) == 0x80 {
            start += 1;
        }
    }
    let end = char_end(&bytes[start..]) + start;
    String::from_utf8_lossy(&bytes[start..end]).into_owned()
}

/// Where the last whole character of `bytes` ends (a character cut off at
/// the end is left out).
pub fn char_end(bytes: &[u8]) -> usize {
    let len = bytes.len();
    // A character is at most 4 bytes: look at the last 3 for a lead byte
    // whose character goes past the end.
    for back in 1..=3.min(len) {
        let b = bytes[len - back];
        if b & 0xC0 == 0x80 {
            continue; // a continuation byte
        }
        let need = if b & 0x80 == 0 {
            1
        } else if b & 0xE0 == 0xC0 {
            2
        } else if b & 0xF0 == 0xE0 {
            3
        } else if b & 0xF8 == 0xF0 {
            4
        } else {
            1
        };
        return if need > back { len - back } else { len };
    }
    len
}

/// One page of a run's output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// The byte it starts at, and the one after its last.
    pub from: u64,
    pub to: u64,
    /// Bytes of output kept.
    pub size: u64,
    pub text: String,
    /// There is more after it.
    pub more: bool,
}

/// At most `max` bytes of the output file at `path` from byte `from` (from
/// the end when negative), starting and ending on a character.
pub fn read_page(path: &Path, from: i64, max: usize) -> std::io::Result<Page> {
    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    // At least 8 bytes: a page always holds a whole character, so it moves on.
    let max = max.clamp(8, MAX_PAGE);
    let start = if from < 0 {
        size.saturating_sub(from.unsigned_abs())
    } else {
        (from as u64).min(size)
    };
    let n = ((size - start) as usize).min(max);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf)?;
    // Start on a character: skip the rest of one cut at `start`.
    let mut skip = 0;
    while skip < buf.len() && skip < 3 && (buf[skip] & 0xC0) == 0x80 && start > 0 {
        skip += 1;
    }
    let reaches_end = start + n as u64 >= size;
    let end = if reaches_end {
        buf.len()
    } else {
        char_end(&buf[skip..]) + skip
    };
    let text = String::from_utf8_lossy(&buf[skip..end]).into_owned();
    let to = start + end as u64;
    Ok(Page {
        from: start + skip as u64,
        to,
        size,
        text,
        more: to < size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_keeps_the_last_bytes_and_the_file_all_of_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let mut o = Output::new(Some(&path), 8);
        o.push(b"hello ");
        o.push(b"world, ");
        o.push(b"again");
        o.flush();
        assert_eq!(o.size(), 18);
        assert_eq!(o.tail(8), "d, again");
        assert_eq!(o.tail(5), "again");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "hello world, again"
        );
        assert!(!o.truncated());
        // A chunk larger than the tail.
        o.push(b"0123456789abcdef");
        assert_eq!(o.tail(100), "89abcdef");
    }

    #[test]
    fn a_tail_never_starts_or_ends_inside_a_character() {
        let mut o = Output::new(None, 4);
        o.push("aé€".as_bytes()); // 1 + 2 + 3 bytes
                                  // The last 4 bytes are é's second byte and €: the half is dropped.
        assert_eq!(o.tail(4), "€");
        let mut o = Output::new(None, 64);
        o.push(&"€".as_bytes()[..2]);
        assert_eq!(o.tail(64), "", "a character still arriving is not shown");
        o.push(&"€".as_bytes()[2..]);
        assert_eq!(o.tail(64), "€");
        assert_eq!(char_end(b"abc"), 3);
        assert_eq!(char_end(&"a€".as_bytes()[..3]), 1);
    }

    #[test]
    fn pages_cover_the_output_without_cutting_characters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let text = "line one\nlíne twö\nthe € end\n".repeat(50);
        std::fs::write(&path, &text).unwrap();
        let mut got = String::new();
        let mut from = 0i64;
        loop {
            let p = read_page(&path, from, 9).unwrap();
            assert_eq!(p.size, text.len() as u64);
            got.push_str(&p.text);
            if !p.more {
                break;
            }
            assert!(p.to > p.from, "a page moves on");
            from = p.to as i64;
        }
        assert_eq!(got, text, "pages read back the whole output");
        // From the end.
        let p = read_page(&path, -9, 100).unwrap();
        assert_eq!(p.text, " € end\n");
        assert!(!p.more);
        // Past the end: nothing.
        let p = read_page(&path, 1 << 40, 100).unwrap();
        assert_eq!((p.text.as_str(), p.more), ("", false));
    }
}
