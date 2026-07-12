//! Capture the compile-time git commit for the wire build identifier.
//!
//! Builds outside a git checkout degrade to `unknown` instead of failing.

use std::path::Path;
use std::process::Command;

fn git(manifest: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_owned());
    let commit = git(&manifest, &["rev-parse", "--short=12", "HEAD"])
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=ORC_BUILD_COMMIT={commit}");
    // Rebuild when HEAD moves so the identifier cannot go stale.
    if let Some(git_dir) = git(&manifest, &["rev-parse", "--absolute-git-dir"]) {
        let head = Path::new(&git_dir).join("HEAD");
        if head.exists() {
            println!("cargo:rerun-if-changed={}", head.display());
        }
        if let Some(reference) = git(&manifest, &["symbolic-ref", "-q", "HEAD"]) {
            let target = Path::new(&git_dir).join(reference);
            if target.exists() {
                println!("cargo:rerun-if-changed={}", target.display());
            }
        }
        let packed = Path::new(&git_dir).join("packed-refs");
        if packed.exists() {
            println!("cargo:rerun-if-changed={}", packed.display());
        }
    }
}
