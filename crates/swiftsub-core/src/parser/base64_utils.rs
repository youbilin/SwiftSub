use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

use crate::error::{Error, Result};

pub fn decode_base64(input: &str) -> Result<Vec<u8>> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Err(Error::InvalidLink("empty base64 payload".into()));
    }

    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(&cleaned) {
            return Ok(bytes);
        }
    }

    Err(Error::InvalidLink("invalid base64 payload".into()))
}

pub fn decode_base64_text(input: &str) -> Result<String> {
    let bytes = decode_base64(input)?;
    String::from_utf8(bytes).map_err(|_| Error::InvalidLink("base64 payload is not UTF-8".into()))
}

pub fn try_decode_base64_text(input: &str) -> Option<String> {
    decode_base64_text(input)
        .ok()
        .filter(|decoded| !decoded.trim().is_empty())
}
