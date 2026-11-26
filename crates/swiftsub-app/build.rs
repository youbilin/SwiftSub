fn main() {
    println!("cargo:rerun-if-changed=swiftsub.rc");
    println!("cargo:rerun-if-changed=../../assets/swiftsub.ico");
    println!("cargo:rerun-if-changed=../../assets/swiftsub-32.rgba");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let res_path = out_dir.join("swiftsub.res");

    let rc_exe = find_rc_exe().expect("rc.exe not found; run with Visual Studio Build Tools");
    let status = std::process::Command::new(rc_exe)
        .args(["/nologo", "/fo"])
        .arg(&res_path)
        .arg("swiftsub.rc")
        .status()
        .expect("failed to execute rc.exe");
    assert!(status.success(), "rc.exe failed to compile swiftsub.rc");

    println!("cargo:rustc-link-arg-bins={}", res_path.display());
}

fn find_rc_exe() -> Option<std::path::PathBuf> {
    if let Ok(output) = std::process::Command::new("where.exe")
        .arg("rc.exe")
        .output()
        && output.status.success()
        && let Some(first) = String::from_utf8_lossy(&output.stdout).lines().next()
    {
        let path = std::path::PathBuf::from(first.trim());
        if path.exists() {
            return Some(path);
        }
    }
    let kits = std::path::PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = std::fs::read_dir(&kits) {
        let mut versions: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        versions.sort();
        versions.reverse();
        for version_dir in versions {
            let candidate = version_dir.join("x64").join("rc.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}
