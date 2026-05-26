fn main() {
    // Retry loop for MSYS2 windres file-lock race on release profile.
    for attempt in 0..5 {
        match tauri_build::try_build(Default::default()) {
            Ok(_) => break,
            Err(e) if attempt < 4 => {
                eprintln!("[attempt {}/5] tauri_build failed: {:#}", attempt + 1, e);
                if let Ok(out_dir) = std::env::var("OUT_DIR") {
                    for name in ["libresource.a", "resource.rc"] {
                        let p = std::path::Path::new(&out_dir).join(name);
                        let _ = std::fs::remove_file(&p);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(2000));
            }
            Err(e) => {
                panic!("tauri_build failed after 5 attempts: {:#}", e);
            }
        }
    }

    // Copy WinRing0 to target directory (workaround for MSYS2 resource bundling issue).
    // The build script runs from src-tauri/ so target is at target/ relative to cwd.
    let resources = std::path::Path::new("resources");
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target = std::path::Path::new("target").join(&profile);

    for file in &["WinRing0x64.dll", "WinRing0x64.sys"] {
        let src = resources.join(file);
        let dst = target.join(file);
        if src.exists() {
            let _ = std::fs::create_dir_all(&target);
            let _ = std::fs::copy(&src, &dst);
        }
    }
}
