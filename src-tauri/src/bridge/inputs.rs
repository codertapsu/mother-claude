//! Turn input: normalization and validation.
//!
//! A client may send `input` as a bare string, one typed item, or an array of
//! them. Everything is normalized to the Anthropic content-block shapes the
//! Agent SDK and the Messages API both accept, because the SDK's streaming
//! input is a `MessageParam` whose `content` is `string | ContentBlockParam[]`
//! — images and documents go straight through.
//!
//! Convenience item types (`localImage`, `localDocument`, bare `url`) are
//! expanded here so a caller never has to base64 anything itself. Validation is
//! deliberately strict and happens *before* the runtime is touched: a bad image
//! must fail as a typed 422/413 from the bridge, not as an opaque
//! "an image could not be processed" sentence from the model.

use std::path::Path;

use serde_json::{json, Value};

use super::error::{BridgeError, BridgeResult};

/// Media types the Messages API accepts for `image` blocks.
pub const IMAGE_MEDIA_TYPES: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];
/// Largest decoded image, per the vision docs.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Largest edge, per the vision docs (images above this are silently resized
/// upstream, which breaks coordinate work — so the bridge refuses instead).
pub const MAX_IMAGE_EDGE: u32 = 8000;
/// Smallest edge. Undocumented upstream, but tiny images are rejected by the
/// API with an untyped error; failing here gives the caller something useful.
pub const MIN_IMAGE_EDGE: u32 = 8;
/// Largest decoded document (PDF / text) inlined into a turn.
pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
/// Largest number of images in one message, per the vision docs.
pub const MAX_IMAGES_PER_MESSAGE: usize = 100;
/// Largest message text, mirroring the Codex bridge's limit.
pub const MAX_TEXT_CHARS: usize = 32_000;

/// Image pixel dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    /// Input tokens this image will cost, per the vision docs' `⌈w/28⌉×⌈h/28⌉`.
    pub fn estimated_tokens(&self) -> u64 {
        let cells = |n: u32| u64::from(n).div_ceil(28);
        cells(self.width) * cells(self.height)
    }
}

/// The normalized content of one user turn.
#[derive(Debug, Clone, Default)]
pub struct NormalizedInput {
    /// Anthropic content blocks, ready to hand to the SDK or the Messages API.
    pub blocks: Vec<Value>,
    /// Concatenated text, for transcripts and log lines.
    pub text: String,
    pub image_count: usize,
    pub estimated_image_tokens: u64,
}

impl NormalizedInput {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn content(&self) -> Value {
        Value::Array(self.blocks.clone())
    }
}

/// Normalize a whole `input` field.
pub fn normalize(input: &Value) -> BridgeResult<NormalizedInput> {
    let items: Vec<&Value> = match input {
        Value::String(_) | Value::Object(_) => vec![input],
        Value::Array(items) => items.iter().collect(),
        Value::Null => {
            return Err(BridgeError::invalid_input(
                "`input` is required: a string, an item object, or an array of items.",
            ))
        }
        other => {
            return Err(BridgeError::invalid_input(format!(
                "`input` must be a string, object or array, not {}.",
                type_name(other)
            )))
        }
    };

    let mut out = NormalizedInput::default();
    for (index, item) in items.into_iter().enumerate() {
        let block = normalize_item(item, index, &mut out)?;
        out.blocks.push(block);
        // Checked inside the loop, not after it: each image is decoded or read
        // from disk as it is processed, so a request with ten thousand of them
        // would otherwise buy gigabytes of memory before being rejected.
        if out.image_count > MAX_IMAGES_PER_MESSAGE {
            return Err(BridgeError::invalid_request(format!(
                "More than {MAX_IMAGES_PER_MESSAGE} images in one message."
            ))
            .with("field", format!("input[{index}]")));
        }
    }

    if out.blocks.is_empty() {
        return Err(BridgeError::invalid_input("`input` produced no content."));
    }
    Ok(out)
}

fn normalize_item(item: &Value, index: usize, out: &mut NormalizedInput) -> BridgeResult<Value> {
    let field = |suffix: &str| format!("input[{index}].{suffix}");

    if let Value::String(text) = item {
        return text_block(text, &field("")).inspect(|_| push_text(out, text));
    }

    let obj = item.as_object().ok_or_else(|| {
        BridgeError::invalid_field(
            &format!("input[{index}]"),
            format!("Expected a string or an object, got {}.", type_name(item)),
        )
    })?;

    let kind = obj.get("type").and_then(Value::as_str).ok_or_else(|| {
        BridgeError::invalid_field(&field("type"), "Each input item needs a `type`.")
    })?;

    match kind {
        "text" => {
            let text = obj.get("text").and_then(Value::as_str).ok_or_else(|| {
                BridgeError::invalid_field(&field("text"), "A text item needs `text`.")
            })?;
            push_text(out, text);
            text_block(text, &field("text"))
        }
        "image" => {
            let source = image_source(obj, index)?;
            out.image_count += 1;
            if let Some(dims) = source.dimensions {
                out.estimated_image_tokens += dims.estimated_tokens();
            }
            Ok(json!({ "type": "image", "source": source.value }))
        }
        "localImage" => {
            let path = obj.get("path").and_then(Value::as_str).ok_or_else(|| {
                BridgeError::invalid_field(
                    &field("path"),
                    "A localImage item needs an absolute `path`.",
                )
            })?;
            let bytes = read_local(path, &field("path"), MAX_IMAGE_BYTES, "image_size")?;
            let media_type = sniff_image(&bytes).ok_or_else(|| {
                BridgeError::unprocessable(
                    "image_format",
                    format!("{path} is not a PNG, JPEG, GIF or WebP image."),
                )
                .with("field", field("path"))
            })?;
            let dims = validate_image(&bytes, media_type, &field("path"))?;
            out.image_count += 1;
            out.estimated_image_tokens += dims.estimated_tokens();
            Ok(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": encode_base64(&bytes),
                }
            }))
        }
        "document" => {
            let mut block = json!({ "type": "document", "source": document_source(obj, index)? });
            for key in ["title", "context", "citations"] {
                if let Some(v) = obj.get(key) {
                    block[key] = v.clone();
                }
            }
            Ok(block)
        }
        "localDocument" => {
            let path = obj.get("path").and_then(Value::as_str).ok_or_else(|| {
                BridgeError::invalid_field(
                    &field("path"),
                    "A localDocument item needs an absolute `path`.",
                )
            })?;
            let bytes = read_local(path, &field("path"), MAX_DOCUMENT_BYTES, "request_size")?;
            let title = obj
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(str::to_string)
                });
            let source = if bytes.starts_with(b"%PDF-") {
                json!({
                    "type": "base64",
                    "media_type": "application/pdf",
                    "data": encode_base64(&bytes),
                })
            } else {
                let text = String::from_utf8(bytes).map_err(|_| {
                    BridgeError::unprocessable(
                        "invalid_document",
                        format!("{path} is neither a PDF nor UTF-8 text."),
                    )
                    .with("field", field("path"))
                })?;
                json!({ "type": "text", "media_type": "text/plain", "data": text })
            };
            let mut block = json!({ "type": "document", "source": source });
            if let Some(title) = title {
                block["title"] = json!(title);
            }
            Ok(block)
        }
        // Anything else is an Anthropic content block we do not need to
        // understand (tool_result, search_result, container_upload, …). Pass it
        // through rather than inventing a whitelist that goes stale.
        _ => Ok(item.clone()),
    }
}

fn push_text(out: &mut NormalizedInput, text: &str) {
    if !out.text.is_empty() {
        out.text.push('\n');
    }
    out.text.push_str(text);
}

fn text_block(text: &str, field: &str) -> BridgeResult<Value> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(BridgeError::unprocessable(
            "invalid_text",
            format!("Text exceeds {MAX_TEXT_CHARS} characters."),
        )
        .with("field", field));
    }
    Ok(json!({ "type": "text", "text": text }))
}

struct ImageSource {
    value: Value,
    dimensions: Option<Dimensions>,
}

/// Accept `{source:{...}}` (native), `{url:"..."}` or `{url:"data:..."}`
/// (Codex-compatible sugar), and `{data, media_type}`.
fn image_source(obj: &serde_json::Map<String, Value>, index: usize) -> BridgeResult<ImageSource> {
    let field = |suffix: &str| format!("input[{index}].{suffix}");

    if let Some(url) = obj.get("url").and_then(Value::as_str) {
        return image_from_url(url, &field("url"));
    }

    let Some(source) = obj.get("source").and_then(Value::as_object) else {
        // `{data, media_type}` shorthand.
        if let Some(data) = obj.get("data").and_then(Value::as_str) {
            let media_type = obj
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            return image_from_base64(data, media_type, &field("data"));
        }
        return Err(BridgeError::invalid_field(
            &field("source"),
            "An image item needs `source`, `url`, or `data` + `media_type`.",
        ));
    };

    match source.get("type").and_then(Value::as_str) {
        Some("base64") => {
            let data = source.get("data").and_then(Value::as_str).ok_or_else(|| {
                BridgeError::invalid_field(&field("source.data"), "A base64 source needs `data`.")
            })?;
            let media_type = source
                .get("media_type")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    BridgeError::invalid_field(
                        &field("source.media_type"),
                        "A base64 source needs `media_type`.",
                    )
                })?;
            image_from_base64(data, media_type, &field("source.data"))
        }
        Some("url") => {
            let url = source.get("url").and_then(Value::as_str).ok_or_else(|| {
                BridgeError::invalid_field(&field("source.url"), "A url source needs `url`.")
            })?;
            image_from_url(url, &field("source.url"))
        }
        // `file` sources reference the workspace-scoped Files API. The docs are
        // explicit that a file_id must never be accepted from an untrusted
        // client, so the bridge refuses to relay one it did not mint.
        Some("file") => Err(BridgeError::forbidden(
            "file_id_not_accepted",
            "Files API `file_id` sources are not accepted from bridge clients; \
             upload through POST /files and use the handle it returns.",
        )
        .with("field", field("source.type"))),
        other => Err(BridgeError::invalid_field(
            &field("source.type"),
            format!(
                "Unsupported image source type {:?}; expected base64 or url.",
                other.unwrap_or("<missing>")
            ),
        )),
    }
}

fn image_from_url(url: &str, field: &str) -> BridgeResult<ImageSource> {
    if let Some((media_type, data)) = parse_data_url(url) {
        return image_from_base64(data, media_type, field);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        // Remote URLs are fetched by Anthropic, not by us — pass through.
        return Ok(ImageSource {
            value: json!({ "type": "url", "url": url }),
            dimensions: None,
        });
    }
    Err(BridgeError::invalid_field(
        field,
        "An image `url` must be a data: URL or an http(s) URL. \
         Use `{\"type\":\"localImage\",\"path\":…}` for a file on this machine.",
    ))
}

fn image_from_base64(data: &str, media_type: &str, field: &str) -> BridgeResult<ImageSource> {
    if !IMAGE_MEDIA_TYPES.contains(&media_type) {
        return Err(BridgeError::unprocessable(
            "image_format",
            format!(
                "Unsupported image media type {media_type:?}; expected one of {}.",
                IMAGE_MEDIA_TYPES.join(", ")
            ),
        )
        .with("field", field));
    }
    // A data: URL may have been handed to us whole.
    let payload = parse_data_url(data).map(|(_, d)| d).unwrap_or(data);
    let bytes = decode_base64(payload).ok_or_else(|| {
        BridgeError::unprocessable("invalid_image", "Image data is not valid base64.")
            .with("field", field)
    })?;

    let sniffed = sniff_image(&bytes).ok_or_else(|| {
        BridgeError::unprocessable(
            "image_format",
            "Image bytes are not a PNG, JPEG, GIF or WebP.",
        )
        .with("field", field)
    })?;
    if sniffed != media_type {
        return Err(BridgeError::unprocessable(
            "image_format",
            format!("Declared media type {media_type:?} but the bytes are {sniffed:?}."),
        )
        .with("field", field));
    }

    let dimensions = validate_image(&bytes, media_type, field)?;
    Ok(ImageSource {
        value: json!({ "type": "base64", "media_type": media_type, "data": encode_base64(&bytes) }),
        dimensions: Some(dimensions),
    })
}

fn document_source(obj: &serde_json::Map<String, Value>, index: usize) -> BridgeResult<Value> {
    let field = format!("input[{index}].source");
    let source = obj
        .get("source")
        .and_then(Value::as_object)
        .ok_or_else(|| BridgeError::invalid_field(&field, "A document item needs `source`."))?;
    match source.get("type").and_then(Value::as_str) {
        Some("base64") | Some("text") | Some("content") | Some("url") => {
            Ok(Value::Object(source.clone()))
        }
        Some("file") => Err(BridgeError::forbidden(
            "file_id_not_accepted",
            "Files API `file_id` sources are not accepted from bridge clients.",
        )
        .with("field", field)),
        other => Err(BridgeError::invalid_field(
            &field,
            format!(
                "Unsupported document source type {:?}; expected base64, text, content or url.",
                other.unwrap_or("<missing>")
            ),
        )),
    }
}

fn read_local(
    path: &str,
    field: &str,
    max: usize,
    over_code: &'static str,
) -> BridgeResult<Vec<u8>> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(BridgeError::invalid_field(field, "Path must be absolute."));
    }
    let meta = std::fs::metadata(p)
        .map_err(|e| BridgeError::invalid_field(field, format!("Cannot read {path}: {e}")))?;
    if meta.len() as usize > max {
        return Err(BridgeError::too_large(
            over_code,
            format!("{path} is {} bytes; the limit is {max}.", meta.len()),
        )
        .with("field", field));
    }
    std::fs::read(p)
        .map_err(|e| BridgeError::invalid_field(field, format!("Cannot read {path}: {e}")))
}

fn validate_image(bytes: &[u8], media_type: &str, field: &str) -> BridgeResult<Dimensions> {
    if bytes.is_empty() {
        return Err(
            BridgeError::unprocessable("invalid_image", "Image is empty.").with("field", field),
        );
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(BridgeError::too_large(
            "image_size",
            format!(
                "Image is {} bytes; the limit is {MAX_IMAGE_BYTES}.",
                bytes.len()
            ),
        )
        .with("field", field));
    }
    let dims = image_dimensions(bytes).ok_or_else(|| {
        BridgeError::unprocessable(
            "invalid_image",
            format!("Could not read {media_type} dimensions."),
        )
        .with("field", field)
    })?;
    if dims.width > MAX_IMAGE_EDGE || dims.height > MAX_IMAGE_EDGE {
        return Err(BridgeError::too_large(
            "image_dimensions",
            format!(
                "Image is {}x{}; each edge must be at most {MAX_IMAGE_EDGE}px. \
                 Resize it yourself so pixel coordinates stay meaningful.",
                dims.width, dims.height
            ),
        )
        .with("field", field));
    }
    if dims.width < MIN_IMAGE_EDGE || dims.height < MIN_IMAGE_EDGE {
        return Err(BridgeError::unprocessable(
            "image_dimensions",
            format!(
                "Image is {}x{}; each edge must be at least {MIN_IMAGE_EDGE}px \
                 or the model rejects it.",
                dims.width, dims.height
            ),
        )
        .with("field", field));
    }
    Ok(dims)
}

/// Split `data:<media-type>[;charset=…];base64,<payload>`.
pub fn parse_data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let meta = meta.strip_suffix(";base64")?;
    let media_type = meta.split(';').next()?;
    Some((media_type, payload))
}

/// Identify an image by magic bytes, so a lying `media_type` cannot slip past.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Read pixel dimensions from an image header without decoding it.
pub fn image_dimensions(bytes: &[u8]) -> Option<Dimensions> {
    match sniff_image(bytes)? {
        "image/png" => png_dimensions(bytes),
        "image/jpeg" => jpeg_dimensions(bytes),
        "image/gif" => gif_dimensions(bytes),
        "image/webp" => webp_dimensions(bytes),
        _ => None,
    }
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn png_dimensions(b: &[u8]) -> Option<Dimensions> {
    // 8-byte signature, 4-byte length, "IHDR", width, height.
    if b.len() < 24 || &b[12..16] != b"IHDR" {
        return None;
    }
    Some(Dimensions {
        width: be_u32(&b[16..20]),
        height: be_u32(&b[20..24]),
    })
}

fn gif_dimensions(b: &[u8]) -> Option<Dimensions> {
    if b.len() < 10 {
        return None;
    }
    Some(Dimensions {
        width: u32::from(u16::from_le_bytes([b[6], b[7]])),
        height: u32::from(u16::from_le_bytes([b[8], b[9]])),
    })
}

fn jpeg_dimensions(b: &[u8]) -> Option<Dimensions> {
    let mut i = 2usize;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        // Standalone markers carry no length.
        if marker == 0xFF || (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        // SOF0-SOF3, SOF5-SOF7, SOF9-SOF11, SOF13-SOF15 carry the frame size;
        // C4 (DHT), C8 (JPG) and CC (DAC) share the range but do not.
        let is_sof =
            (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC;
        if is_sof {
            if i + 9 >= b.len() {
                return None;
            }
            return Some(Dimensions {
                height: u32::from(u16::from_be_bytes([b[i + 5], b[i + 6]])),
                width: u32::from(u16::from_be_bytes([b[i + 7], b[i + 8]])),
            });
        }
        if len < 2 {
            return None;
        }
        i += 2 + len;
    }
    None
}

fn webp_dimensions(b: &[u8]) -> Option<Dimensions> {
    if b.len() < 16 {
        return None;
    }
    match &b[12..16] {
        b"VP8 " => {
            // 8-byte chunk header, 3-byte frame tag, 3-byte sync code.
            if b.len() < 30 || b[23..26] != [0x9D, 0x01, 0x2A] {
                return None;
            }
            Some(Dimensions {
                width: u32::from(u16::from_le_bytes([b[26], b[27]]) & 0x3FFF),
                height: u32::from(u16::from_le_bytes([b[28], b[29]]) & 0x3FFF),
            })
        }
        b"VP8L" => {
            if b.len() < 25 || b[20] != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
            Some(Dimensions {
                width: (bits & 0x3FFF) + 1,
                height: ((bits >> 14) & 0x3FFF) + 1,
            })
        }
        b"VP8X" => {
            if b.len() < 30 {
                return None;
            }
            let le24 = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], 0]);
            Some(Dimensions {
                width: le24(&b[24..27]) + 1,
                height: le24(&b[27..30]) + 1,
            })
        }
        _ => None,
    }
}

// --- base64 -------------------------------------------------------------
//
// Hand-rolled so the bridge does not take a dependency for ~40 lines, and so
// decoding is strict: whitespace is tolerated (clients wrap data: URLs), but
// any other stray character is an error rather than a silent truncation.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => continue,
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1x1 PNG, then padded out so the dimension guard can be exercised
    /// separately from the format guard.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b
    }

    #[test]
    fn base64_round_trips() {
        for case in [
            &b""[..],
            &b"f"[..],
            &b"fo"[..],
            &b"foo"[..],
            &b"foob"[..],
            &b"fooba"[..],
            &b"foobar"[..],
        ] {
            let encoded = encode_base64(case);
            assert_eq!(decode_base64(&encoded).unwrap(), case, "{encoded}");
        }
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
        // Whitespace is tolerated; other junk is not.
        assert_eq!(decode_base64("Zm9v\nYmFy").unwrap(), b"foobar");
        assert!(decode_base64("Zm9v*YmFy").is_none());
    }

    #[test]
    fn data_urls_split_correctly() {
        assert_eq!(
            parse_data_url("data:image/png;base64,AAAA"),
            Some(("image/png", "AAAA"))
        );
        assert_eq!(
            parse_data_url("data:text/plain;charset=utf-8;base64,QQ=="),
            Some(("text/plain", "QQ=="))
        );
        assert_eq!(parse_data_url("https://example.test/a.png"), None);
        // Non-base64 data URLs are not supported.
        assert_eq!(parse_data_url("data:image/png,raw"), None);
    }

    #[test]
    fn dimensions_are_read_from_every_supported_header() {
        assert_eq!(
            image_dimensions(&png(640, 480)),
            Some(Dimensions {
                width: 640,
                height: 480
            })
        );

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&200u16.to_le_bytes());
        gif.extend_from_slice(&100u16.to_le_bytes());
        assert_eq!(
            image_dimensions(&gif),
            Some(Dimensions {
                width: 200,
                height: 100
            })
        );

        // JPEG: SOI, an APP0 segment to skip, then SOF0.
        let mut jpg = vec![0xFF, 0xD8];
        jpg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        jpg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        jpg.extend_from_slice(&300u16.to_be_bytes()); // height
        jpg.extend_from_slice(&400u16.to_be_bytes()); // width
        jpg.extend_from_slice(&[0x03, 0x01, 0x22, 0x00]);
        assert_eq!(
            image_dimensions(&jpg),
            Some(Dimensions {
                width: 400,
                height: 300
            })
        );

        // WebP lossy (VP8 ).
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&0u32.to_le_bytes());
        webp.extend_from_slice(b"WEBPVP8 ");
        webp.extend_from_slice(&0u32.to_le_bytes());
        webp.extend_from_slice(&[0, 0, 0]); // frame tag
        webp.extend_from_slice(&[0x9D, 0x01, 0x2A]);
        webp.extend_from_slice(&128u16.to_le_bytes());
        webp.extend_from_slice(&64u16.to_le_bytes());
        assert_eq!(
            image_dimensions(&webp),
            Some(Dimensions {
                width: 128,
                height: 64
            })
        );

        // WebP extended (VP8X) stores canvas size minus one.
        let mut vp8x = b"RIFF".to_vec();
        vp8x.extend_from_slice(&0u32.to_le_bytes());
        vp8x.extend_from_slice(b"WEBPVP8X");
        vp8x.extend_from_slice(&10u32.to_le_bytes());
        vp8x.extend_from_slice(&[0, 0, 0, 0]);
        vp8x.extend_from_slice(&[0x0F, 0x00, 0x00]); // width - 1 = 15
        vp8x.extend_from_slice(&[0x07, 0x00, 0x00]); // height - 1 = 7
        assert_eq!(
            image_dimensions(&vp8x),
            Some(Dimensions {
                width: 16,
                height: 8
            })
        );
    }

    #[test]
    fn token_estimate_matches_the_documented_formula() {
        // ceil(1000/28) * ceil(1000/28) = 36 * 36
        assert_eq!(
            Dimensions {
                width: 1000,
                height: 1000
            }
            .estimated_tokens(),
            36 * 36
        );
    }

    #[test]
    fn a_bare_string_becomes_one_text_block() {
        let out = normalize(&json!("hello")).unwrap();
        assert_eq!(out.blocks.len(), 1);
        assert_eq!(out.blocks[0]["type"], "text");
        assert_eq!(out.text, "hello");
    }

    #[test]
    fn mixed_arrays_normalize_and_count_images() {
        let data = encode_base64(&png(200, 100));
        let out = normalize(&json!([
            { "type": "text", "text": "look" },
            { "type": "image", "url": format!("data:image/png;base64,{data}") },
        ]))
        .unwrap();
        assert_eq!(out.blocks.len(), 2);
        assert_eq!(out.image_count, 1);
        assert_eq!(out.blocks[1]["source"]["media_type"], "image/png");
        assert!(out.estimated_image_tokens > 0);
        assert_eq!(out.text, "look");
    }

    #[test]
    fn a_lying_media_type_is_rejected() {
        let data = encode_base64(&png(200, 100));
        let err = normalize(&json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/jpeg", "data": data }
        }))
        .unwrap_err();
        assert_eq!(err.code, "image_format");
    }

    #[test]
    fn oversized_and_undersized_images_are_refused() {
        let big = encode_base64(&png(9000, 10));
        let err = normalize(&json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": big }
        }))
        .unwrap_err();
        assert_eq!(err.code, "image_dimensions");
        assert_eq!(err.status, axum::http::StatusCode::PAYLOAD_TOO_LARGE);

        let tiny = encode_base64(&png(2, 2));
        let err = normalize(&json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": tiny }
        }))
        .unwrap_err();
        assert_eq!(err.code, "image_dimensions");
        assert_eq!(err.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn file_id_sources_are_never_relayed() {
        let err = normalize(&json!({
            "type": "image",
            "source": { "type": "file", "file_id": "file_abc" }
        }))
        .unwrap_err();
        assert_eq!(err.code, "file_id_not_accepted");
        assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn unknown_block_types_pass_through_untouched() {
        let block = json!({ "type": "tool_result", "tool_use_id": "t1", "content": "ok" });
        let out = normalize(&block).unwrap();
        assert_eq!(out.blocks[0], block);
    }

    #[test]
    fn empty_and_wrongly_typed_input_is_rejected() {
        assert_eq!(normalize(&Value::Null).unwrap_err().code, "invalid_input");
        assert_eq!(normalize(&json!(7)).unwrap_err().code, "invalid_input");
        assert_eq!(normalize(&json!([])).unwrap_err().code, "invalid_input");
        assert_eq!(
            normalize(&json!({ "text": "no type" })).unwrap_err().code,
            "invalid_field"
        );
    }

    #[test]
    fn overlong_text_is_rejected_by_character_count() {
        let err = normalize(&json!("é".repeat(MAX_TEXT_CHARS + 1))).unwrap_err();
        assert_eq!(err.code, "invalid_text");
    }

    #[test]
    fn http_image_urls_pass_through_for_anthropic_to_fetch() {
        let out =
            normalize(&json!({ "type": "image", "url": "https://example.test/a.png" })).unwrap();
        assert_eq!(out.blocks[0]["source"]["type"], "url");
        assert_eq!(out.image_count, 1);
    }

    #[test]
    fn relative_local_paths_are_refused() {
        let err = normalize(&json!({ "type": "localImage", "path": "a.png" })).unwrap_err();
        assert_eq!(err.code, "invalid_field");
    }
}
