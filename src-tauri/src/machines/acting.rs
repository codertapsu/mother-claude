//! Who acts behind a proxied request (docs/PLAN-MACHINES.md §5.5).
//!
//! When the desktop or a paired phone drives a session on another computer
//! through this one, the request carries `X-MC-Acting`: who acts here and
//! whether that person holds the dangerous right *here*. The other computer
//! trusts its paired peer to say so, audits both names, and allows a
//! dangerous action only when the peer has its `dangerous` grant **and** the
//! acting person had the right on the peer — so a phone without the grant
//! on the Mac cannot reach for it through the Mac.

use crate::server::context::{ClientContext, Principal};
use crate::state::AppState;

pub const HEADER: &str = "x-mc-acting";

/// Who acts, as the target reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acting {
    /// `desktop`, `device <name>`, `script`, or `machine` (no header: the
    /// peer itself, e.g. its hello).
    pub label: String,
    /// The acting person had the dangerous right on the peer.
    pub dangerous: bool,
}

/// The header this machine sends for `ctx`.
pub fn header_for(state: &AppState, ctx: &ClientContext) -> String {
    let dangerous = u8::from(ctx.may_dangerous());
    match &ctx.principal {
        Principal::Desktop => "desktop".to_string(),
        Principal::Device { id, .. } => {
            let name = state.devices.get(id).map(|d| d.name).unwrap_or_default();
            format!(
                "device; id={}; dangerous={dangerous}; name={}",
                encode(id),
                encode(&name)
            )
        }
        Principal::LocalScript => format!("script; dangerous={dangerous}"),
        _ => format!("other; dangerous={dangerous}"),
    }
}

/// Reads the header (absent or malformed: the peer itself, nothing dangerous).
pub fn parse(value: Option<&str>) -> Acting {
    let Some(v) = value
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 512)
    else {
        return Acting {
            label: "machine".into(),
            dangerous: false,
        };
    };
    let mut parts = v.split(';').map(str::trim);
    let kind = parts.next().unwrap_or_default();
    let mut dangerous = false;
    let mut name = String::new();
    for p in parts {
        if let Some((k, val)) = p.split_once('=') {
            match k.trim() {
                "dangerous" => dangerous = val.trim() == "1",
                "name" => name = decode(val.trim()),
                _ => {}
            }
        }
    }
    match kind {
        "desktop" => Acting {
            label: "desktop".into(),
            dangerous: true,
        },
        "device" => Acting {
            label: if name.is_empty() {
                "device".into()
            } else {
                format!(
                    "device {}",
                    name.chars()
                        .filter(|c| !c.is_control())
                        .take(64)
                        .collect::<String>()
                )
            },
            dangerous,
        },
        "script" => Acting {
            label: "script".into(),
            dangerous,
        },
        _ => Acting {
            label: "machine".into(),
            dangerous: false,
        },
    }
}

/// Percent-encodes everything but unreserved characters (a name may hold
/// anything, a header only ASCII).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_round_trips_and_defaults_to_nothing_dangerous() {
        let a = parse(Some(
            "device; id=dev_1; dangerous=1; name=My%20Phone%20%E2%9C%93",
        ));
        assert_eq!(a.label, "device My Phone ✓");
        assert!(a.dangerous);
        let a = parse(Some("device; id=dev_1; dangerous=0; name=x"));
        assert!(!a.dangerous);
        assert!(parse(Some("desktop")).dangerous);
        assert!(!parse(Some("script; dangerous=0")).dangerous);
        let none = parse(None);
        assert_eq!(none.label, "machine");
        assert!(!none.dangerous);
        assert!(
            !parse(Some("root; dangerous=1")).dangerous,
            "unknown kinds are never dangerous"
        );
        assert!(!parse(Some(&"x".repeat(600))).dangerous);
        assert_eq!(decode(&encode("a b;c=d%")), "a b;c=d%");
    }
}
