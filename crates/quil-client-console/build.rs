use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=QCLIENT_CONSOLE_BUILD_COMMIT");
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    watch_git_path(&manifest, "HEAD");
    if let Some(reference) = git_output(&manifest, &["symbolic-ref", "-q", "HEAD"]) {
        watch_git_path(&manifest, &reference);
    }
    if let Ok(commit) = std::env::var("QCLIENT_CONSOLE_BUILD_COMMIT") {
        let commit = commit.trim();
        assert!(!commit.is_empty(), "QCLIENT_CONSOLE_BUILD_COMMIT is empty");
        println!("cargo:rustc-env=QCLIENT_CONSOLE_BUILD_COMMIT={commit}");
        return;
    }
    if let Some(commit) = git_output(&manifest, &["rev-parse", "--short=12", "HEAD"]) {
        println!("cargo:rustc-env=QCLIENT_CONSOLE_BUILD_COMMIT={commit}");
    }
}

fn watch_git_path(manifest: &str, name: &str) {
    if let Some(path) = git_output(manifest, &["rev-parse", "--git-path", name]) {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn git_output(manifest: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
