fn main() {
    let args: Vec<_> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).unwrap();
    let rules = swiftsub_core::generator::decode_mrs_payload(&bytes, &args[2]).unwrap();
    let text = rules.join("\n");
    if let Some(path) = args.get(3) {
        std::fs::write(path, text).unwrap();
    } else {
        println!("{text}");
    }
    eprintln!("Decoded {} {} rules", rules.len(), args[2]);
}
