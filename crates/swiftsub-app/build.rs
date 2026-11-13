fn main() {
    // Native GPUI view construction nests several unoptimized Rust render frames.
    // Windows defaults to a 1 MiB main-thread stack; reserve 8 MiB (committed on demand).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bin=swiftsub=/STACK:8388608");
    }
}
