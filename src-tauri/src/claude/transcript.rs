//! Append-only JSONL transcript reading.
//!
//! A [`TranscriptTailer`] keeps a byte offset into one `<session-id>.jsonl` file
//! and, on each [`poll`](TranscriptTailer::poll), returns the events appended
//! since the last call. Lines may be flushed partially, so bytes are buffered
//! and only split on `\n` once a full line is available. UTF-8 boundaries that
//! fall mid-multibyte across reads are handled by buffering raw bytes.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::schema::{parse_transcript_line, TranscriptEvent};

/// Stateful tailer for a single transcript file.
#[derive(Debug)]
pub struct TranscriptTailer {
    path: PathBuf,
    offset: u64,
    /// Trailing bytes of an incomplete final line, carried to the next poll.
    partial: Vec<u8>,
}

impl TranscriptTailer {
    /// Tail from the beginning of the file (replays full history on first poll).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            partial: Vec::new(),
        }
    }

    /// Tail starting at the current end of file, so only *new* lines are
    /// returned (history is skipped). Falls back to offset 0 if the file is not
    /// yet readable.
    pub fn at_end(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self {
            path,
            offset,
            partial: Vec::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read everything appended since the last poll and parse complete lines.
    ///
    /// Detects truncation/rotation (file shorter than our offset) and restarts
    /// from the beginning. Returns an empty vec when there is nothing new or the
    /// file does not yet exist.
    pub fn poll(&mut self) -> std::io::Result<Vec<TranscriptEvent>> {
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };

        let len = file.metadata()?.len();
        if len < self.offset {
            // Truncated or rotated — restart.
            self.offset = 0;
            self.partial.clear();
        }
        if len == self.offset {
            return Ok(Vec::new());
        }

        file.seek(SeekFrom::Start(self.offset))?;
        let mut fresh = Vec::new();
        let read = file.read_to_end(&mut fresh)?;
        self.offset += read as u64;

        // Combine carried partial bytes with the freshly read bytes.
        let mut buf = std::mem::take(&mut self.partial);
        buf.extend_from_slice(&fresh);

        let mut events = Vec::new();
        let mut start = 0usize;
        for i in 0..buf.len() {
            if buf[i] == b'\n' {
                let line = &buf[start..i];
                let text = String::from_utf8_lossy(line);
                if let Some(ev) = parse_transcript_line(&text) {
                    events.push(ev);
                }
                start = i + 1;
            }
        }
        // Whatever follows the last newline is an incomplete line — carry it.
        self.partial = buf[start..].to_vec();

        Ok(events)
    }
}

/// Read and parse an entire transcript file in one shot (used by REST reads of
/// historical transcripts). Missing files yield an empty vec.
pub fn read_all(path: impl AsRef<Path>) -> std::io::Result<Vec<TranscriptEvent>> {
    match std::fs::read_to_string(path.as_ref()) {
        Ok(blob) => Ok(super::schema::parse_transcript(&blob)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn append(path: &Path, line: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(line.as_bytes()).unwrap();
    }

    #[test]
    fn tails_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");

        let mut tailer = TranscriptTailer::new(&path);
        assert!(tailer.poll().unwrap().is_empty()); // file does not exist yet

        append(&path, "{\"type\":\"user\"}\n");
        let ev = tailer.poll().unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].event_type, "user");

        // No new bytes -> nothing.
        assert!(tailer.poll().unwrap().is_empty());

        append(&path, "{\"type\":\"assistant\"}\n{\"type\":\"system\"}\n");
        let ev = tailer.poll().unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[1].event_type, "system");
    }

    #[test]
    fn buffers_partial_lines_until_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let mut tailer = TranscriptTailer::new(&path);

        // Write a line in two flushes; the first has no trailing newline.
        append(&path, "{\"type\":\"assi");
        assert!(tailer.poll().unwrap().is_empty()); // incomplete -> buffered

        append(&path, "stant\"}\n");
        let ev = tailer.poll().unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].event_type, "assistant");
    }

    #[test]
    fn handles_truncation_by_restarting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let mut tailer = TranscriptTailer::new(&path);

        append(&path, "{\"type\":\"assistant\"}\n"); // 21 bytes -> offset 21
        assert_eq!(tailer.poll().unwrap().len(), 1);

        // Genuine truncation: the new file is shorter than our offset, which is
        // the realistic rotation signal for an append-only log.
        std::fs::write(&path, "{\"type\":\"x\"}\n").unwrap(); // 13 bytes < 21
        let ev = tailer.poll().unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].event_type, "x");
    }

    #[test]
    fn at_end_skips_existing_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        append(&path, "{\"type\":\"user\"}\n");

        let mut tailer = TranscriptTailer::at_end(&path);
        assert!(tailer.poll().unwrap().is_empty()); // history skipped

        append(&path, "{\"type\":\"assistant\"}\n");
        assert_eq!(tailer.poll().unwrap().len(), 1);
    }

    #[test]
    fn read_all_parses_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        append(
            &path,
            "{\"type\":\"user\"}\ngarbage\n{\"type\":\"assistant\"}\n",
        );
        let events = read_all(&path).unwrap();
        assert_eq!(events.len(), 2);
    }
}
