//! Robustness and fuzz-style adversarial edge-case tests for parsers, MRS decoding,
//! share-link export, and sing-box generation. Guarantees zero panics on hostile input.

use base64::Engine as _;
use swiftsub_core::generator::{decode_mrs_payload, generate_singbox_json};
use swiftsub_core::normalize::build_nodes;
use swiftsub_core::parser::parse_content;
use swiftsub_core::{GenerateOptions, generate_profile};

#[test]
fn hostile_and_malformed_inputs_never_panic() {
    let long_url = format!("vless://{}@example.org:443#long", "a".repeat(32_768));
    let huge_base64 = base64::engine::general_purpose::STANDARD.encode("not-a-link\n".repeat(4000));
    let nested_b64 = {
        let mut s = "trojan://pass@example.org:443#ok".to_string();
        for _ in 0..5 {
            s = base64::engine::general_purpose::STANDARD.encode(&s);
        }
        s
    };

    let cases: Vec<&str> = vec![
        "",
        "   \r\n\t  ",
        "\u{feff}",
        "://",
        "vless://",
        "vless://@",
        "vless://uuid@[::1:443#bad-ipv6",
        "vless://uuid@example.org:99999#port-overflow",
        "vless://uuid@example.org:0#zero-port",
        "vless://uuid@example.org:443?extra=%7Bbad-json#xhttp",
        "vmess://not-valid-base64!!!",
        "vmess://e30=",
        "ss://",
        "ss://@@@:80#broken",
        "ssr://!!!",
        "trojan://@:0#",
        "hysteria://example.org:0",
        "hysteria2://@:abc",
        "tuic://:@example.org:443",
        "juicity://onlyuser@example.org:443",
        "wireguard://bad@example.org:51820?publickey=123",
        "proxies: 123",
        "proxies:\n  - null\n  - 42\n  - {}\n",
        "proxies: [",
        "{\"proxies\": \"not-a-list\"}",
        "unknown-proto://user:pass@127.0.0.1:80#x",
        &long_url,
        &huge_base64,
        &nested_b64,
    ];

    for input in cases {
        let _ = parse_content(input);
    }
}

#[test]
fn malformed_mrs_payloads_are_rejected_without_panic() {
    let empty: &[u8] = b"";
    let garbage: &[u8] = &[0xff, 0x00, 0x12, 0x34, 0x56, 0x78];
    let zstd_empty = zstd::stream::encode_all(empty, 1).unwrap();
    let zstd_truncated = zstd::stream::encode_all(&b"MRS!"[..], 1).unwrap();
    let zstd_bad_magic = zstd::stream::encode_all(&b"NOT_MRS_PAYLOAD_12345"[..], 1).unwrap();

    for payload in [
        empty,
        garbage,
        &zstd_empty,
        &zstd_truncated,
        &zstd_bad_magic,
    ] {
        for behavior in ["domain", "ipcidr", "classical"] {
            assert!(decode_mrs_payload(payload, behavior).is_err());
        }
    }
}

#[test]
fn special_characters_in_node_names_survive_yaml_and_singbox_export() {
    let links = "\
trojan://p%40ss%3A123@example.org:443?sni=example.org#HK%20%5B%E4%B8%93%E7%BA%BF%5D%20%22Quote%22%20%27Single%27
hysteria://example.org:443?auth=secret&peer=example.org&upmbps=50&downmbps=100&obfs=xplus#Hy1-Node
juicity://11111111-1111-1111-1111-111111111111:jpass@juicity.example.org:443?sni=juicity.example.org&congestion_control=bbr#Juicity-Node";
    let parsed = parse_content(links).unwrap();
    assert_eq!(parsed.proxies.len(), 3);
    assert_eq!(parsed.proxies[1].kind(), "hysteria");
    assert_eq!(parsed.proxies[2].kind(), "juicity");

    let nodes = build_nodes(parsed.proxies, "1", "sub");
    let mihomo = generate_profile(&nodes, &GenerateOptions::default()).unwrap();
    let reparsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&mihomo.yaml).unwrap();
    assert_eq!(reparsed["proxies"].as_sequence().unwrap().len(), 3);

    // sing-box exports the supported subset cleanly without failing on hysteria v1 / juicity
    let sb_json = generate_singbox_json(&nodes, &GenerateOptions::default()).unwrap();
    let sb_doc: serde_json::Value = serde_json::from_str(&sb_json).unwrap();
    assert!(sb_doc["outbounds"].as_array().unwrap().len() >= 3);
}
