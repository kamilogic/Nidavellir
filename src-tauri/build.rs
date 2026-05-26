fn main() {
    tauri_build::build();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target = std::path::Path::new(&manifest).join("target").join(&profile);

    for name in &["WinRing0x64.dll", "WinRing0x64.sys"] {
        let src = std::path::Path::new(&manifest).join("resources").join(name);
        let dst = target.join(name);
        if src.exists() {
            let _ = std::fs::copy(&src, &dst);
            println!("cargo:warning=Resource {name} copied to {:?}", target);
        } else {
            println!("cargo:warning=Resource {name} not found, skipping");
        }
    }
}
