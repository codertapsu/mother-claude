//! Tolerant, allocation-light serde building blocks for transcript lines.
//!
//! Transcript lines reach 35 MB (base64 images, `originalFile`, huge tool
//! results). Deserializing them into `serde_json::Value` would allocate the
//! whole tree; these wrappers instead:
//!
//! * accept any JSON type and fall back to "absent" on a type mismatch (a
//!   version that changes a field's type never fails the whole record);
//! * skip everything they do not need via `IgnoredAny` (no allocation);
//! * cap strings ([`CapStr`]) and count instead of copying ([`LineCount`],
//!   [`PatchCount`], [`KeyCount`]).
//!
//! Every wrapper implements `Default`, so record structs can use
//! `#[serde(default)]` and unknown keys are ignored.

use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, DeserializeOwned, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

/// Drain the rest of a map.
fn drain_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

/// Drain the rest of a sequence.
fn drain_seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<(), A::Error> {
    while seq.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

/// Truncate at a char boundary.
pub fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

macro_rules! scalar_visitor_rest {
    ($t:ty) => {
        fn visit_none<E: de::Error>(self) -> Result<$t, E> {
            Ok(<$t>::default())
        }
        fn visit_unit<E: de::Error>(self) -> Result<$t, E> {
            Ok(<$t>::default())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<$t, A::Error> {
            drain_seq(seq)?;
            Ok(<$t>::default())
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<$t, A::Error> {
            drain_map(map)?;
            Ok(<$t>::default())
        }
    };
}

/// A string, or absent for any other JSON type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LStr(pub Option<String>);

impl LStr {
    pub fn get(&self) -> Option<&str> {
        self.0.as_deref()
    }
    pub fn take(self) -> Option<String> {
        self.0
    }
    pub fn is(&self, v: &str) -> bool {
        self.0.as_deref() == Some(v)
    }
}

impl<'de> Deserialize<'de> for LStr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LStr;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<LStr, E> {
                Ok(LStr(Some(v.to_string())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<LStr, E> {
                Ok(LStr(Some(v)))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<LStr, E> {
                Ok(LStr(None))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<LStr, E> {
                Ok(LStr(Some(v.to_string())))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<LStr, E> {
                Ok(LStr(Some(v.to_string())))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<LStr, E> {
                Ok(LStr(None))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<LStr, D::Error> {
                LStr::deserialize(d)
            }
            scalar_visitor_rest!(LStr);
        }
        d.deserialize_any(V)
    }
}

/// A string capped at `N` bytes (on a char boundary); absent for other types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapStr<const N: usize> {
    pub text: Option<String>,
    /// The original was longer than `N`.
    pub cut: bool,
    /// Original length in bytes.
    pub len: usize,
}

impl<const N: usize> CapStr<N> {
    pub fn get(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

impl<'de, const N: usize> Deserialize<'de> for CapStr<N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<const M: usize>;
        impl<'de, const M: usize> Visitor<'de> for V<M> {
            type Value = CapStr<M>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<CapStr<M>, E> {
                let kept = clip(v, M);
                Ok(CapStr {
                    text: Some(kept.to_string()),
                    cut: kept.len() < v.len(),
                    len: v.len(),
                })
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<CapStr<M>, E> {
                Ok(CapStr::default())
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<CapStr<M>, E> {
                Ok(CapStr::default())
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<CapStr<M>, E> {
                Ok(CapStr::default())
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<CapStr<M>, E> {
                Ok(CapStr::default())
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<CapStr<M>, D::Error> {
                CapStr::<M>::deserialize(d)
            }
            scalar_visitor_rest!(CapStr<M>);
        }
        d.deserialize_any(V::<N>)
    }
}

/// A non-negative integer (floats truncated, negative/other types absent).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LU64(pub Option<u64>);

impl LU64 {
    pub fn n(&self) -> u64 {
        self.0.unwrap_or(0)
    }
}

impl<'de> Deserialize<'de> for LU64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LU64;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<LU64, E> {
                Ok(LU64(Some(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<LU64, E> {
                Ok(LU64((v >= 0).then_some(v as u64)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<LU64, E> {
                Ok(LU64((v.is_finite() && v >= 0.0).then_some(v as u64)))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<LU64, E> {
                Ok(LU64(None))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<LU64, E> {
                Ok(LU64(v.trim().parse().ok()))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<LU64, D::Error> {
                LU64::deserialize(d)
            }
            scalar_visitor_rest!(LU64);
        }
        d.deserialize_any(V)
    }
}

/// A float.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LF64(pub Option<f64>);

impl<'de> Deserialize<'de> for LF64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LF64;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<LF64, E> {
                Ok(LF64(Some(v as f64)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<LF64, E> {
                Ok(LF64(Some(v as f64)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<LF64, E> {
                Ok(LF64(v.is_finite().then_some(v)))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<LF64, E> {
                Ok(LF64(None))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<LF64, E> {
                Ok(LF64(v.trim().parse().ok()))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<LF64, D::Error> {
                LF64::deserialize(d)
            }
            scalar_visitor_rest!(LF64);
        }
        d.deserialize_any(V)
    }
}

/// A boolean (absent for other types).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LBool(pub Option<bool>);

impl LBool {
    pub fn yes(&self) -> bool {
        self.0 == Some(true)
    }
}

impl<'de> Deserialize<'de> for LBool {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LBool;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<LBool, E> {
                Ok(LBool(Some(v)))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<LBool, E> {
                Ok(LBool(None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<LBool, E> {
                Ok(LBool(None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<LBool, E> {
                Ok(LBool(None))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<LBool, E> {
                Ok(LBool(None))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<LBool, D::Error> {
                LBool::deserialize(d)
            }
            scalar_visitor_rest!(LBool);
        }
        d.deserialize_any(V)
    }
}

/// Present and not null (value ignored).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Present(pub bool);

impl<'de> Deserialize<'de> for Present {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Present;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Present, E> {
                Ok(Present(true))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Present, E> {
                Ok(Present(true))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Present, E> {
                Ok(Present(true))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Present, E> {
                Ok(Present(true))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Present, E> {
                Ok(Present(true))
            }
            fn visit_none<E: de::Error>(self) -> Result<Present, E> {
                Ok(Present(false))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Present, E> {
                Ok(Present(false))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Present, D::Error> {
                Present::deserialize(d)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Present, A::Error> {
                drain_seq(seq)?;
                Ok(Present(true))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Present, A::Error> {
                drain_map(map)?;
                Ok(Present(true))
            }
        }
        d.deserialize_any(V)
    }
}

/// An object deserialized as `T`, or absent for any other type.
#[derive(Debug, Clone, PartialEq)]
pub struct Obj<T>(pub Option<T>);

impl<T> Default for Obj<T> {
    fn default() -> Self {
        Obj(None)
    }
}

impl<T> Obj<T> {
    pub fn get(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Obj<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Obj<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Obj<T>, A::Error> {
                T::deserialize(de::value::MapAccessDeserializer::new(map)).map(|t| Obj(Some(t)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Obj<T>, A::Error> {
                drain_seq(seq)?;
                Ok(Obj(None))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Obj<T>, E> {
                Ok(Obj(None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Obj<T>, E> {
                Ok(Obj(None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Obj<T>, E> {
                Ok(Obj(None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Obj<T>, E> {
                Ok(Obj(None))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Obj<T>, E> {
                Ok(Obj(None))
            }
            fn visit_none<E: de::Error>(self) -> Result<Obj<T>, E> {
                Ok(Obj(None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Obj<T>, E> {
                Ok(Obj(None))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Obj<T>, D::Error> {
                Obj::<T>::deserialize(d)
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// A sequence of `T` (elements must themselves be tolerant), capped at
/// `MAX_SEQ` kept elements; empty for any other type.
#[derive(Debug, Clone, PartialEq)]
pub struct Seq<T>(pub Vec<T>);

impl<T> Default for Seq<T> {
    fn default() -> Self {
        Seq(Vec::new())
    }
}

/// Elements kept per [`Seq`].
pub const MAX_SEQ: usize = 4096;

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Seq<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Seq<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Seq<T>, A::Error> {
                let mut out = Vec::new();
                while out.len() < MAX_SEQ {
                    match seq.next_element::<T>()? {
                        Some(t) => out.push(t),
                        None => return Ok(Seq(out)),
                    }
                }
                drain_seq(seq)?;
                Ok(Seq(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Seq<T>, A::Error> {
                drain_map(map)?;
                Ok(Seq(Vec::new()))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Seq<T>, E> {
                Ok(Seq(Vec::new()))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Seq<T>, E> {
                Ok(Seq(Vec::new()))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Seq<T>, E> {
                Ok(Seq(Vec::new()))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Seq<T>, E> {
                Ok(Seq(Vec::new()))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Seq<T>, E> {
                Ok(Seq(Vec::new()))
            }
            fn visit_none<E: de::Error>(self) -> Result<Seq<T>, E> {
                Ok(Seq(Vec::new()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Seq<T>, E> {
                Ok(Seq(Vec::new()))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Seq<T>, D::Error> {
                Seq::<T>::deserialize(d)
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// Number of lines in a string value (0 for other types).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineCount(pub u64);

impl<'de> Deserialize<'de> for LineCount {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LineCount;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<LineCount, E> {
                if v.is_empty() {
                    return Ok(LineCount(0));
                }
                let n = v.bytes().filter(|b| *b == b'\n').count() as u64;
                Ok(LineCount(if v.ends_with('\n') { n } else { n + 1 }))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<LineCount, E> {
                Ok(LineCount(0))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<LineCount, E> {
                Ok(LineCount(0))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<LineCount, E> {
                Ok(LineCount(0))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<LineCount, E> {
                Ok(LineCount(0))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<LineCount, D::Error> {
                LineCount::deserialize(d)
            }
            scalar_visitor_rest!(LineCount);
        }
        d.deserialize_any(V)
    }
}

/// `+`/`-` line counts of a `structuredPatch` (array of hunks with `lines`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PatchCount {
    pub added: u64,
    pub removed: u64,
    pub hunks: u64,
}

macro_rules! scalar_rest_lines {
    () => {
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Lines, A::Error> {
            drain_map(map)?;
            Ok(Lines(PatchCount::default()))
        }
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Lines, E> {
            Ok(Lines(PatchCount::default()))
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Lines, E> {
            Ok(Lines(PatchCount::default()))
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Lines, E> {
            Ok(Lines(PatchCount::default()))
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Lines, E> {
            Ok(Lines(PatchCount::default()))
        }
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Lines, E> {
            Ok(Lines(PatchCount::default()))
        }
        fn visit_unit<E: de::Error>(self) -> Result<Lines, E> {
            Ok(Lines(PatchCount::default()))
        }
    };
}

struct HunkLines(PatchCount);

impl<'de> Deserialize<'de> for HunkLines {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Lines(PatchCount);
        impl<'de> Deserialize<'de> for Lines {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> Visitor<'de> for V {
                    type Value = Lines;
                    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                        f.write_str("lines")
                    }
                    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Lines, A::Error> {
                        let mut c = PatchCount::default();
                        while let Some(line) = seq.next_element::<CapStr<1>>()? {
                            match line.get() {
                                Some("+") => c.added += 1,
                                Some("-") => c.removed += 1,
                                _ => {}
                            }
                        }
                        Ok(Lines(c))
                    }
                    scalar_rest_lines!();
                }
                d.deserialize_any(V)
            }
        }
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = HunkLines;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("hunk")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<HunkLines, A::Error> {
                let mut c = PatchCount::default();
                while let Some(key) = map.next_key::<CapStr<16>>()? {
                    if key.get() == Some("lines") {
                        c = map.next_value::<Lines>()?.0;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(HunkLines(c))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<HunkLines, A::Error> {
                drain_seq(seq)?;
                Ok(HunkLines(PatchCount::default()))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<HunkLines, E> {
                Ok(HunkLines(PatchCount::default()))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<HunkLines, E> {
                Ok(HunkLines(PatchCount::default()))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<HunkLines, E> {
                Ok(HunkLines(PatchCount::default()))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<HunkLines, E> {
                Ok(HunkLines(PatchCount::default()))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<HunkLines, E> {
                Ok(HunkLines(PatchCount::default()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<HunkLines, E> {
                Ok(HunkLines(PatchCount::default()))
            }
        }
        d.deserialize_any(V)
    }
}

impl<'de> Deserialize<'de> for PatchCount {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let hunks = Seq::<HunkLines>::deserialize(d)?;
        let mut c = PatchCount::default();
        for h in hunks.0 {
            c.added += h.0.added;
            c.removed += h.0.removed;
            c.hunks += 1;
        }
        Ok(c)
    }
}

/// The keys of an object (values skipped), capped at `MAX_SEQ`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keys(pub Vec<String>);

impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Keys;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("anything")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
                let mut out = Vec::new();
                while let Some(k) = map.next_key::<CapStr<4096>>()? {
                    map.next_value::<IgnoredAny>()?;
                    if out.len() < MAX_SEQ {
                        if let Some(k) = k.text {
                            out.push(k);
                        }
                    }
                }
                Ok(Keys(out))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Keys, A::Error> {
                drain_seq(seq)?;
                Ok(Keys::default())
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Keys, E> {
                Ok(Keys::default())
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Keys, E> {
                Ok(Keys::default())
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Keys, E> {
                Ok(Keys::default())
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Keys, E> {
                Ok(Keys::default())
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Keys, E> {
                Ok(Keys::default())
            }
            fn visit_unit<E: de::Error>(self) -> Result<Keys, E> {
                Ok(Keys::default())
            }
        }
        d.deserialize_any(V)
    }
}

/// A `serde_json::Value` kept only when its JSON text is at most `N` bytes
/// (larger values are skipped without being materialised twice).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmallValue<const N: usize>(pub Option<serde_json::Value>);

impl<'de, const N: usize> Deserialize<'de> for SmallValue<N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        if v.is_null() {
            return Ok(SmallValue(None));
        }
        let size = serde_json::to_vec(&v)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        Ok(SmallValue((size <= N).then_some(v)))
    }
}

/// Parse one line into `T`; `None` (logged at trace) on malformed JSON.
pub fn parse_line<T: DeserializeOwned>(line: &[u8]) -> Option<T> {
    let line = trim_line(line);
    if line.is_empty() {
        return None;
    }
    match serde_json::from_slice::<T>(line) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::trace!(error = %e, "unparseable transcript line skipped");
            None
        }
    }
}

/// Strip a trailing `\n` / `\r\n`.
pub fn trim_line(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b'\n' || line[end - 1] == b'\r') {
        end -= 1;
    }
    &line[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    struct T {
        a: LStr,
        b: LU64,
        c: LBool,
        d: Obj<Inner>,
        e: Seq<LStr>,
        f: CapStr<3>,
        g: LineCount,
        h: PatchCount,
        k: Keys,
        p: Present,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    struct Inner {
        x: LU64,
    }

    #[test]
    fn wrong_types_never_fail_the_record() {
        let t: T = serde_json::from_str(
            r#"{"a":{"deep":[1,2,{"z":3}]},"b":"x","c":"yes","d":[1],"e":{"k":1},"f":7,"g":{},"h":"x","k":[1],"p":null,"zzz":1}"#,
        )
        .unwrap();
        assert_eq!(t.a.0, None);
        assert_eq!(t.b.0, None);
        assert_eq!(t.c.0, None);
        assert!(t.d.0.is_none());
        assert!(t.e.0.is_empty());
        assert!(t.f.text.is_none());
        assert_eq!(t.g.0, 0);
        assert_eq!(t.h, PatchCount::default());
        assert!(t.k.0.is_empty());
        assert!(!t.p.0);
    }

    #[test]
    fn right_types_are_read() {
        let t: T = serde_json::from_str(
            r#"{"a":"hi","b":12,"c":true,"d":{"x":3,"y":"n"},"e":["p",1,"q"],"f":"héllo","g":"a\nb\n","h":[{"oldStart":1,"lines":[" x","-y","+z","+w"]},{"lines":["-q"]}],"k":{"one":1,"two":{"x":2}},"p":{}}"#,
        )
        .unwrap();
        assert_eq!(t.a.get(), Some("hi"));
        assert_eq!(t.b.n(), 12);
        assert!(t.c.yes());
        assert_eq!(t.d.get().unwrap().x.n(), 3);
        assert_eq!(t.e.0.len(), 3);
        assert_eq!(t.e.0[1].0, Some("1".to_string()));
        assert_eq!(t.f.get(), Some("hé"));
        assert!(t.f.cut);
        assert_eq!(t.g.0, 2);
        assert_eq!(
            t.h,
            PatchCount {
                added: 2,
                removed: 2,
                hunks: 2
            }
        );
        assert_eq!(t.k.0, vec!["one".to_string(), "two".to_string()]);
        assert!(t.p.0);
    }

    #[test]
    fn small_value_drops_big_values() {
        let v: SmallValue<10> = serde_json::from_str(r#"{"a":"b"}"#).unwrap();
        assert!(v.0.is_some());
        let v: SmallValue<10> = serde_json::from_str(r#"{"a":"bbbbbbbbbbbbbbbb"}"#).unwrap();
        assert!(v.0.is_none());
    }
}
