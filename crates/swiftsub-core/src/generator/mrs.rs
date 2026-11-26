use crate::error::{Error, Result};
use std::io::{Cursor, Read};

const LIMIT: usize = 64 * 1024 * 1024;
const TEXT_LIMIT: usize = 10 * 1024 * 1024;

fn invalid() -> Error {
    Error::InvalidLink("MRS 内容无效、版本不支持或超过读取上限".into())
}
fn bytes<'a>(reader: &mut Cursor<&'a [u8]>, length: usize) -> Result<&'a [u8]> {
    let start = reader.position() as usize;
    let end = start.checked_add(length).ok_or_else(invalid)?;
    let data = reader.get_ref().get(start..end).ok_or_else(invalid)?;
    reader.set_position(end as u64);
    Ok(data)
}
fn number(reader: &mut Cursor<&[u8]>) -> Result<u64> {
    Ok(u64::from_be_bytes(
        bytes(reader, 8)?.try_into().map_err(|_| invalid())?,
    ))
}
fn length(reader: &mut Cursor<&[u8]>) -> Result<usize> {
    let value = usize::try_from(number(reader)?).map_err(|_| invalid())?;
    if value > LIMIT {
        return Err(invalid());
    }
    Ok(value)
}
fn bit(words: &[u64], index: usize) -> Result<bool> {
    Ok(words.get(index / 64).ok_or_else(invalid)? & (1 << (index % 64)) != 0)
}
fn words(reader: &mut Cursor<&[u8]>) -> Result<Vec<u64>> {
    let count = length(reader)?;
    let data = bytes(reader, count.checked_mul(8).ok_or_else(invalid)?)?;
    Ok(data
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| u64::from_be_bytes(*chunk))
        .collect())
}

/// Decode MRS v1's Zstandard envelope and binary domain trie or inclusive IP ranges.
/// Reconstructed domain patterns retain exact versus subdomain-only matching.
pub fn decode_mrs_payload(content: &[u8], behavior: &str) -> Result<Vec<String>> {
    let decoder = zstd::stream::read::Decoder::new(content).map_err(|_| invalid())?;
    let mut decoded = Vec::new();
    decoder
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|_| invalid())?;
    if decoded.len() > LIMIT {
        return Err(invalid());
    }
    let mut reader = Cursor::new(decoded.as_slice());
    if bytes(&mut reader, 4)? != b"MRS\x01" {
        return Err(invalid());
    }
    let kind = bytes(&mut reader, 1)?[0];
    if !matches!((behavior, kind), ("domain", 0) | ("ipcidr", 1)) {
        return Err(Error::InvalidLink("MRS 内容类型与规则集设置不一致".into()));
    }
    let _source_count = number(&mut reader)?;
    let reserved = length(&mut reader)?;
    bytes(&mut reader, reserved)?;
    if bytes(&mut reader, 1)? != [1] {
        return Err(invalid());
    }
    let payload = if kind == 0 {
        domains(&mut reader)?
    } else {
        networks(&mut reader)?
    };
    if reader.position() as usize != decoded.len() {
        return Err(invalid());
    }
    super::parse_rule_set_payload(&payload.join("\n"), behavior)
}

fn domains(reader: &mut Cursor<&[u8]>) -> Result<Vec<String>> {
    let leaves = words(reader)?;
    let bitmap = words(reader)?;
    let count = length(reader)?;
    let labels = bytes(reader, count)?;
    let mut parents = vec![0usize; count + 1];
    let mut position = 0;
    let mut edge = 0;
    for node in 0..=count {
        while !bit(&bitmap, position)? {
            if edge >= count || edge < node {
                return Err(invalid());
            }
            parents[edge + 1] = node;
            edge += 1;
            position += 1;
        }
        position += 1;
    }
    if edge != count {
        return Err(invalid());
    }
    let mut payload = Vec::new();
    let mut size = 0;
    for node in 1..=count {
        if !bit(&leaves, node)? {
            continue;
        }
        let mut path = Vec::new();
        let mut current = node;
        while current > 0 {
            path.push(labels[current - 1]);
            current = parents[current];
            if path.len() > 4096 {
                return Err(invalid());
            }
        }
        path.reverse();
        let reversed = String::from_utf8(path).map_err(|_| invalid())?;
        let pattern: String = reversed
            .strip_suffix('+')
            .unwrap_or(&reversed)
            .chars()
            .rev()
            .collect();
        size += pattern.len() + 1;
        if size > TEXT_LIMIT {
            return Err(invalid());
        }
        payload.push(pattern);
    }
    Ok(payload)
}

fn networks(reader: &mut Cursor<&[u8]>) -> Result<Vec<String>> {
    let count = length(reader)?;
    let ranges = bytes(reader, count.checked_mul(32).ok_or_else(invalid)?)?;
    let mut payload = Vec::new();
    let mut size = 0;
    for range in ranges.as_chunks::<32>().0 {
        let mut from = u128::from_be_bytes(range[..16].try_into().unwrap());
        let to = u128::from_be_bytes(range[16..].try_into().unwrap());
        if from > to {
            return Err(invalid());
        }
        let ipv4 = from >> 32 == 0xffff && to >> 32 == 0xffff;
        let width = if ipv4 { 32 } else { 128 };
        loop {
            let aligned = from.trailing_zeros().min(width);
            let available = to
                .checked_sub(from)
                .and_then(|difference| difference.checked_add(1));
            let fit = available
                .map(|remaining| 127 - remaining.leading_zeros())
                .unwrap_or(128);
            let bits = aligned.min(fit);
            let ip = if ipv4 {
                std::net::IpAddr::V4(std::net::Ipv4Addr::from(from as u32))
            } else {
                std::net::IpAddr::V6(std::net::Ipv6Addr::from(from))
            };
            let entry = format!("{ip}/{}", width - bits);
            size += entry.len() + 1;
            if size > TEXT_LIMIT {
                return Err(invalid());
            }
            payload.push(entry);
            if bits == 128 {
                break;
            }
            match from.checked_add(1u128 << bits) {
                Some(next) if next <= to => from = next,
                _ => break,
            }
        }
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn reads_mihomo_generated_domains_and_ranges_without_changing_matching_semantics() {
        let fixture = |text: &str| {
            base64::engine::general_purpose::STANDARD
                .decode(text.trim())
                .unwrap()
        };
        // These fixtures were produced by Mihomo convert-ruleset from authored YAML.
        let domains = fixture(include_str!("../../tests/fixtures/domain-rules.mrs.base64"));
        let mut rules = decode_mrs_payload(&domains, "domain").unwrap();
        rules.sort();
        assert_eq!(
            rules,
            [
                "*.wild.example.com",
                ".example.net",
                ".only.example.com",
                "example.net",
                "example.org"
            ]
        );
        assert!(decode_mrs_payload(&domains, "ipcidr").is_err());
        assert!(decode_mrs_payload(&domains[..domains.len() / 2], "domain").is_err());
        let ips = fixture(include_str!("../../tests/fixtures/ip-rules.mrs.base64"));
        assert_eq!(
            decode_mrs_payload(&ips, "ipcidr").unwrap(),
            ["192.0.2.0/24", "198.51.100.1/32", "2001:db8::/32"]
        );
    }
}
