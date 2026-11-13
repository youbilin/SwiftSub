use swiftsub_core::{
    GenerateOptions, catalog::Preset, generate_profile, normalize::build_nodes,
    parser::parse_content,
};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        4,
        "usage: validation_profile INPUT OUTPUT MIXED_PORT"
    );
    let input = std::fs::read_to_string(&args[1]).unwrap();
    let parsed = parse_content(&input).unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let nodes = build_nodes(parsed.proxies, "controlled-test", "loopback");
    let options = GenerateOptions {
        preset: Preset::Minimal,
        enabled_modules: Some(vec![]),
        mixed_port: args[3].parse().unwrap(),
        test_url: "http://127.0.0.1:1/health".into(),
        ..Default::default()
    };
    let profile = generate_profile(&nodes, &options).unwrap();
    std::fs::write(&args[2], profile.yaml).unwrap();
}
