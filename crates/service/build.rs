use std::path::Path;
use std::process::Command;

fn git_output(repo: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=NIDAVELLIR_BUILD_REVISION");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    for path in [
        "build.rs",
        "src",
        "../core/src",
        "../gpu-stress/src",
        "../gpu-nvapi/src",
        "../../Cargo.toml",
        "../../Cargo.lock",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let repo = manifest_dir.join("../..");
    let revision = std::env::var("NIDAVELLIR_BUILD_REVISION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            let mut revision = git_output(&repo, &["rev-parse", "HEAD"])?;
            let dirty = git_output(
                &repo,
                &[
                    "status",
                    "--porcelain",
                    "--untracked-files=normal",
                    "--ignore-submodules",
                    "--",
                    "crates/service",
                    "crates/core",
                    "crates/gpu-stress",
                    "crates/gpu-nvapi",
                    "Cargo.toml",
                    "Cargo.lock",
                ],
            )
            .is_some_and(|status| !status.is_empty());
            if dirty {
                revision.push_str("-dirty");
            }
            Some(revision)
        })
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=NIDAVELLIR_BUILD_REVISION={revision}");
}
