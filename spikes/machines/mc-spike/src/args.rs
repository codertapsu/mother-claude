//! A tiny argument parser: one command, positionals, `--flag value`, `--flag=value`
//! and the boolean flags listed below.

use std::collections::HashMap;

use anyhow::{bail, Result};

const BOOL_FLAGS: &[&str] = &["open", "relay-only"];

pub struct Args {
    pub cmd: String,
    pub pos: Vec<String>,
    flags: HashMap<String, Vec<String>>,
}

impl Args {
    pub fn parse(it: impl Iterator<Item = String>) -> Result<Self> {
        let mut it = it.peekable();
        let cmd = it.next().unwrap_or_default();
        let mut pos = Vec::new();
        let mut flags: HashMap<String, Vec<String>> = HashMap::new();
        while let Some(a) = it.next() {
            let Some(name) = a.strip_prefix("--") else {
                pos.push(a);
                continue;
            };
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (name.to_string(), None),
            };
            if BOOL_FLAGS.contains(&name.as_str()) {
                flags.entry(name).or_default().push("true".into());
                continue;
            }
            let v = match inline {
                Some(v) => v,
                None => match it.next() {
                    Some(v) => v,
                    None => bail!("--{name} needs a value"),
                },
            };
            flags.entry(name).or_default().push(v);
        }
        Ok(Self { cmd, pos, flags })
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.flags.get(k).and_then(|v| v.last()).map(String::as_str)
    }

    pub fn all(&self, k: &str) -> Vec<String> {
        self.flags.get(k).cloned().unwrap_or_default()
    }

    pub fn has(&self, k: &str) -> bool {
        self.flags.contains_key(k)
    }

    pub fn num(&self, k: &str, default: u64) -> Result<u64> {
        Ok(match self.get(k) {
            Some(v) => v.parse()?,
            None => default,
        })
    }
}
