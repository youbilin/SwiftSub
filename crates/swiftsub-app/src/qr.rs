//! Pure-Rust QR code SVG encoder for LAN subscription sharing and single-node export.
use qrcodegen::{QrCode, QrCodeEcc};

pub struct EncodedQr {
    pub svg: String,
    pub size: usize,
    pub runs: Vec<(usize, usize, usize)>,
}

/// Render `content` into both a crisp monochrome SVG and merged horizontal dark runs
/// for zero-allocation-lifetime native GPUI rendering.
pub fn encode_qr(content: &str) -> Result<EncodedQr, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("二维码内容不能为空".into());
    }
    let qr = QrCode::encode_text(trimmed, QrCodeEcc::Medium)
        .map_err(|error| format!("内容过长，无法生成二维码：{error}"))?;
    let size = qr.size();
    let border: i32 = 4;
    let total = size + border * 2;
    let mut svg = String::with_capacity((size as usize) * (size as usize) * 8);
    let mut runs = Vec::new();
    use std::fmt::Write as _;
    let _ = write!(
        &mut svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total} {total}" shape-rendering="crispEdges"><rect width="100%" height="100%" fill="#ffffff"/><path fill="#111827" d=""##
    );
    for y in 0..size {
        let mut x = 0;
        while x < size {
            if qr.get_module(x, y) {
                let start = x;
                while x < size && qr.get_module(x, y) {
                    x += 1;
                }
                let len = x - start;
                runs.push((y as usize, start as usize, len as usize));
                let _ = write!(
                    &mut svg,
                    "M{},{}h{}v1h-{}z",
                    start + border,
                    y + border,
                    len,
                    len
                );
            } else {
                x += 1;
            }
        }
    }
    svg.push_str(r##""/></svg>"##);
    Ok(EncodedQr {
        svg,
        size: size as usize,
        runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_subscription_url_to_valid_svg_and_runs() {
        let encoded = encode_qr("http://192.168.1.50:17899/sub/abc123/abc123.yaml").unwrap();
        assert!(encoded.svg.starts_with("<svg "));
        assert!(encoded.svg.contains("fill=\"#ffffff\""));
        assert!(encoded.svg.contains("fill=\"#111827\""));
        assert!(encoded.svg.ends_with("</svg>"));
        assert!(encoded.size >= 21);
        assert!(!encoded.runs.is_empty());
    }

    #[test]
    fn rejects_empty_qr_payload() {
        assert!(encode_qr("   ").is_err());
    }
}
