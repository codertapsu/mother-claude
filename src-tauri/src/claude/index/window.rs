//! Token usage of a session between two times — what one task cost
//! (docs/ROUTING.md) — read from the session's transcripts: the main one
//! and every subagent's, as the index found them.
//!
//! Lines are read one at a time with a bounded buffer (a line longer than
//! 16 MB is a tool result, never usage, and is skipped unread); a line is
//! parsed only when it mentions `"usage"`, into a type that keeps nothing
//! but the message id, model, timestamp and usage. Each API message counts
//! once, as in the index: its records share the message id, the last
//! record's usage wins, and the first record's time places it in or out of
//! the window. Placeholder (`<synthetic>`) and API-error records carry no
//! usage. Costs are estimates from [`crate::claude::pricing`].

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::agg::{parse_ts, UsageRec};
use super::loose::{LBool, LStr, Obj};
use crate::claude::pricing::{self, UsageDetail};

/// Lines longer than this are skipped without being kept in memory.
const LINE_MAX: usize = 16 * 1024 * 1024;
/// At most this much is read across the files of one window.
const READ_MAX: u64 = 2 * 1024 * 1024 * 1024;

/// Usage of one model within the window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub messages: u64,
    /// Estimated USD; `None` when the model has no known price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Usage of a session's API messages within a window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    /// Cache writes (5-minute and 1-hour together).
    pub cache_write: u64,
    pub messages: u64,
    /// Estimated USD of the messages with a known price.
    pub cost_usd: f64,
    /// Messages whose model has no known price (not in `cost_usd`).
    pub unpriced: u64,
    /// By model id.
    pub models: BTreeMap<String, ModelUsage>,
    /// Something could not be read: a file, an over-long line, the cap.
    pub partial: bool,
}

impl WindowUsage {
    /// Every token: input, output, cache reads and writes.
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Line {
    #[serde(rename = "type")]
    ty: LStr,
    timestamp: LStr,
    #[serde(rename = "isApiErrorMessage")]
    is_api_error: LBool,
    message: Obj<LineMsg>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LineMsg {
    id: LStr,
    model: LStr,
    usage: Obj<UsageRec>,
}

/// One API message: where its first record lies, and its last usage.
enum Seen {
    Outside,
    Inside { model: String, usage: UsageDetail },
}

fn detail_of(u: &UsageRec) -> UsageDetail {
    UsageDetail {
        input: u.input_tokens.n(),
        output: u.output_tokens.n(),
        cache_read: u.cache_read_input_tokens.n(),
        cache_creation: u.cache_creation_input_tokens.n(),
        cache_1h: u
            .cache_creation
            .get()
            .map(|c| c.ephemeral_1h_input_tokens.n())
            .unwrap_or(0),
        web_search: u
            .server_tool_use
            .get()
            .map(|s| s.web_search_requests.n())
            .unwrap_or(0),
        us_geo: u.inference_geo.is("us"),
        standard_speed: u.speed.get().is_none_or(|s| s == "standard"),
    }
}

/// The next line into `buf` (without the newline): `Some(true)` when kept,
/// `Some(false)` when longer than `max` (consumed, not kept), `None` at the
/// end of the input.
fn next_line(
    reader: &mut impl BufRead,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<Option<bool>> {
    buf.clear();
    let mut kept = true;
    let mut any = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(any.then_some(kept));
        }
        any = true;
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i, true),
            None => (chunk.len(), false),
        };
        if kept && buf.len() + take <= max {
            buf.extend_from_slice(&chunk[..take]);
        } else {
            kept = false;
            buf.clear();
        }
        reader.consume(take + usize::from(done));
        if done {
            return Ok(Some(kept));
        }
    }
}

fn mentions_usage(line: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"\"usage\"";
    line.windows(NEEDLE.len()).any(|w| w == NEEDLE)
}

/// Read one transcript into `seen`; returns the bytes read and whether a
/// line had to be skipped.
fn read_into(
    reader: impl std::io::Read,
    from_ms: i64,
    to_ms: i64,
    budget: u64,
    seen: &mut HashMap<String, Seen>,
) -> std::io::Result<(u64, bool)> {
    let mut reader = BufReader::with_capacity(64 * 1024, reader);
    let mut buf = Vec::new();
    let mut read: u64 = 0;
    let mut skipped = false;
    while let Some(kept) = next_line(&mut reader, &mut buf, LINE_MAX)? {
        read += buf.len() as u64 + 1;
        if !kept {
            skipped = true;
            continue;
        }
        if read > budget {
            skipped = true;
            break;
        }
        if !mentions_usage(&buf) {
            continue;
        }
        let Ok(line) = serde_json::from_slice::<Line>(&buf) else {
            continue;
        };
        if !line.ty.is("assistant") || line.is_api_error.yes() {
            continue;
        }
        let Some(msg) = line.message.get() else {
            continue;
        };
        let (Some(id), Some(usage)) = (msg.id.get(), msg.usage.get()) else {
            continue;
        };
        let model = msg.model.get().unwrap_or_default();
        if pricing::is_synthetic(model) {
            continue;
        }
        let detail = detail_of(usage);
        match seen.get_mut(id) {
            Some(Seen::Outside) => {}
            Some(Seen::Inside { usage, .. }) => *usage = detail,
            None => {
                let at = parse_ts(line.timestamp.get());
                let inside = at.is_some_and(|t| t >= from_ms && t <= to_ms);
                seen.insert(
                    id.to_string(),
                    if inside {
                        Seen::Inside {
                            model: model.to_string(),
                            usage: detail,
                        }
                    } else {
                        Seen::Outside
                    },
                );
            }
        }
    }
    Ok((read, skipped))
}

/// Usage of the API messages that started within `[from_ms, to_ms]` in
/// `files` (a session's main transcript and its subagents'). Blocking: run
/// it off the async runtime.
pub fn usage_between<P: AsRef<Path>>(files: &[P], from_ms: i64, to_ms: i64) -> WindowUsage {
    let mut seen: HashMap<String, Seen> = HashMap::new();
    let mut out = WindowUsage::default();
    let mut budget = READ_MAX;
    for path in files {
        let path = path.as_ref();
        match std::fs::File::open(path) {
            Ok(f) => match read_into(f, from_ms, to_ms, budget, &mut seen) {
                Ok((read, skipped)) => {
                    budget = budget.saturating_sub(read);
                    out.partial |= skipped;
                }
                Err(e) => {
                    tracing::debug!(path = %path.display(), error = %e, "transcript read failed");
                    out.partial = true;
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "transcript not readable");
                out.partial = true;
            }
        }
    }
    for s in seen.into_values() {
        let Seen::Inside { model, usage } = s else {
            continue;
        };
        let cost = pricing::message_cost(&model, &usage);
        let m = out.models.entry(model).or_default();
        m.input += usage.input;
        m.output += usage.output;
        m.cache_read += usage.cache_read;
        m.cache_write += usage.cache_creation;
        m.messages += 1;
        match cost {
            Some(c) => {
                m.cost_usd = Some(m.cost_usd.unwrap_or(0.0) + c);
                out.cost_usd += c;
            }
            None => out.unpriced += 1,
        }
        out.input += usage.input;
        out.output += usage.output;
        out.cache_read += usage.cache_read;
        out.cache_write += usage.cache_creation;
        out.messages += 1;
    }
    out.cost_usd = (out.cost_usd * 1e6).round() / 1e6;
    for m in out.models.values_mut() {
        m.cost_usd = m.cost_usd.map(|c| (c * 1e6).round() / 1e6);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    const T0: i64 = 1_790_000_000_000;

    fn ts(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms)
            .unwrap()
            .to_rfc3339()
    }

    fn msg(
        id: &str,
        model: &str,
        at: i64,
        input: u64,
        output: u64,
        read: u64,
        write: u64,
    ) -> String {
        json!({
            "type": "assistant",
            "timestamp": ts(at),
            "message": {
                "id": id, "model": model, "role": "assistant",
                "content": [{ "type": "text", "text": "ok" }],
                "usage": {
                    "input_tokens": input, "output_tokens": output,
                    "cache_read_input_tokens": read, "cache_creation_input_tokens": write
                }
            }
        })
        .to_string()
    }

    fn write(dir: &Path, name: &str, lines: &[String]) -> std::path::PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        p
    }

    #[test]
    fn counts_each_message_once_within_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.jsonl",
            &[
                // Before the window.
                msg("m0", "claude-sonnet-5-5", T0 - 60_000, 100, 10, 0, 0),
                json!({ "type": "user", "timestamp": ts(T0), "message": { "role": "user", "content": "go" } }).to_string(),
                // Two records of one message: the last usage wins.
                msg("m1", "claude-sonnet-5-5", T0 + 1_000, 1_000, 5, 0, 0),
                msg("m1", "claude-sonnet-5-5", T0 + 2_000, 1_000, 500, 0, 0),
                msg("m2", "claude-sonnet-5-5", T0 + 3_000, 200, 300, 50_000, 4_000),
                // Placeholder and API error records carry no usage.
                msg("m3", "<synthetic>", T0 + 4_000, 9, 9, 0, 0),
                json!({ "type": "assistant", "isApiErrorMessage": true, "timestamp": ts(T0 + 4_500),
                        "message": { "id": "m4", "model": "claude-sonnet-5-5",
                                     "usage": { "input_tokens": 7 } } }).to_string(),
                "{ torn line".to_string(),
                // After the window.
                msg("m5", "claude-sonnet-5-5", T0 + 120_000, 100, 10, 0, 0),
            ],
        );
        let sub = write(
            dir.path(),
            "agent-a.jsonl",
            &[msg("s1", "claude-haiku-4-5", T0 + 5_000, 2_000, 100, 0, 0)],
        );
        let missing = dir.path().join("gone.jsonl");
        let u = usage_between(&[main, sub, missing], T0, T0 + 60_000);
        assert_eq!(u.messages, 3);
        assert_eq!(u.input, 1_000 + 200 + 2_000);
        assert_eq!(u.output, 500 + 300 + 100);
        assert_eq!(u.cache_read, 50_000);
        assert_eq!(u.cache_write, 4_000);
        assert!(!u.partial, "a missing file is not an unread one");
        // Sonnet 5.5 $2/$10, cache read $0.20, 5-minute write $2.50;
        // Haiku 4.5 $1/$5.
        let sonnet = (1_200.0 * 2.0 + 800.0 * 10.0 + 50_000.0 * 0.2 + 4_000.0 * 2.5) / 1e6;
        let haiku = (2_000.0 * 1.0 + 100.0 * 5.0) / 1e6;
        assert!(
            (u.cost_usd - (sonnet + haiku)).abs() < 1e-9,
            "{}",
            u.cost_usd
        );
        assert_eq!(u.models.len(), 2);
        assert_eq!(u.models["claude-sonnet-5-5"].messages, 2);
        assert_eq!(u.tokens(), 3_200 + 900 + 50_000 + 4_000);
    }

    #[test]
    fn an_unpriced_model_and_an_over_long_line_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut long = String::from("{\"type\":\"user\",\"x\":\"");
        long.push_str(&"a".repeat(LINE_MAX + 10));
        long.push_str("\"}");
        let p = write(
            dir.path(),
            "t.jsonl",
            &[long, msg("m1", "gpt-x", T0 + 1, 10, 10, 0, 0)],
        );
        let u = usage_between(&[p], T0, T0 + 1_000);
        assert!(u.partial);
        assert_eq!(u.messages, 1);
        assert_eq!(u.unpriced, 1);
        assert_eq!(u.cost_usd, 0.0);
        assert_eq!(u.models["gpt-x"].cost_usd, None);
    }

    #[test]
    fn lines_are_split_across_buffer_boundaries() {
        let text = format!("{}\n{}", "x".repeat(70_000), "short");
        let mut r = BufReader::with_capacity(1024, text.as_bytes());
        let mut buf = Vec::new();
        assert_eq!(next_line(&mut r, &mut buf, 100_000).unwrap(), Some(true));
        assert_eq!(buf.len(), 70_000);
        assert_eq!(next_line(&mut r, &mut buf, 100_000).unwrap(), Some(true));
        assert_eq!(buf, b"short");
        assert_eq!(next_line(&mut r, &mut buf, 100_000).unwrap(), None);
        let mut r = BufReader::with_capacity(16, "0123456789abcdef0123\nok\n".as_bytes());
        assert_eq!(next_line(&mut r, &mut buf, 8).unwrap(), Some(false));
        assert_eq!(next_line(&mut r, &mut buf, 8).unwrap(), Some(true));
        assert_eq!(buf, b"ok");
    }
}
