use std::{path::PathBuf, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let result = Command::new("git").args(args).output().ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=RSQL_BUILD_REVISION");
    println!("cargo:rerun-if-changed=build.rs");
    for reference in [
        Some("HEAD".into()),
        git(&["symbolic-ref", "-q", "HEAD"]),
        Some("packed-refs".into()),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference])
            && PathBuf::from(&path).exists()
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(revision) = std::env::var("RSQL_BUILD_REVISION")
        .ok()
        .or_else(|| git(&["rev-parse", "HEAD"]))
        && matches!(revision.len(), 40 | 64)
        && revision.bytes().all(|b| b.is_ascii_hexdigit())
    {
        println!("cargo:rustc-env=RSQL_REVISION={revision}");
    }
}
