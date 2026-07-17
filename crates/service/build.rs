use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const REVISION_PATHS: &[&str] = &[
    "crates/service",
    "crates/core",
    "crates/gpu-stress",
    "crates/gpu-nvapi",
    "Cargo.toml",
    "Cargo.lock",
];

fn git_bytes(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn git_output(repo: &Path, args: &[&str]) -> Option<String> {
    git_bytes(repo, args)
        .map(|output| String::from_utf8_lossy(&output).trim().to_owned())
}

fn hash_revision_payload(repo: &Path, payload: &[u8]) -> Option<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(payload).ok()?;
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn dirty_revision_suffix(repo: &Path) -> Option<String> {
    let mut status_args = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--ignore-submodules",
        "--",
    ];
    status_args.extend_from_slice(REVISION_PATHS);
    let status = git_bytes(repo, &status_args)?;
    if status.is_empty() {
        return Some(String::new());
    }

    let mut diff_args = vec!["diff", "--binary", "HEAD", "--"];
    diff_args.extend_from_slice(REVISION_PATHS);
    let mut payload = git_bytes(repo, &diff_args).unwrap_or_default();
    payload.extend_from_slice(&status);
    for entry in status.split(|byte| *byte == 0) {
        if let Some(path) = entry.strip_prefix(b"?? ") {
            let path = String::from_utf8_lossy(path);
            payload.extend_from_slice(path.as_bytes());
            if let Ok(contents) = std::fs::read(repo.join(path.as_ref())) {
                payload.extend_from_slice(&(contents.len() as u64).to_le_bytes());
                payload.extend_from_slice(&contents);
            }
        }
    }

    let hash = hash_revision_payload(repo, &payload)?;
    Some(format!("-dirty-{}", &hash[..hash.len().min(12)]))
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
            revision.push_str(&dirty_revision_suffix(&repo).unwrap_or_else(|| "-dirty-unknown".into()));
            Some(revision)
        })
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=NIDAVELLIR_BUILD_REVISION={revision}");
}
