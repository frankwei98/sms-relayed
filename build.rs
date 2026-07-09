use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=frontend/dist");
    emit_build_version();

    let dist = Path::new("frontend/dist");
    if dist.exists() {
        return;
    }

    let fallback = dist.join("index.html");
    fs::create_dir_all(dist).expect("failed to create frontend/dist fallback directory");
    fs::write(
        fallback,
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>sms-relayed</title>
</head>
<body>
  <p>The web frontend has not been built. Run <code>pnpm --dir frontend build</code> before packaging a release.</p>
</body>
</html>
"#,
    )
    .expect("failed to write frontend/dist fallback");
}

fn emit_build_version() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Some(head_ref) = git_head_ref() {
        println!("cargo:rerun-if-changed=.git/{head_ref}");
    }

    let package_version =
        std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION should be set by cargo");
    let commit = git_head_commit()
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=SMS_RELAYED_BUILD_VERSION={package_version}+{commit}");
}

fn git_head_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let commit = String::from_utf8(output.stdout).ok()?;
    let commit = commit.trim();
    if commit.is_empty() {
        None
    } else {
        Some(commit.to_string())
    }
}

fn git_head_ref() -> Option<String> {
    let head = fs::read_to_string(".git/HEAD").ok()?;
    head.strip_prefix("ref: ")
        .map(str::trim)
        .filter(|head_ref| !head_ref.is_empty())
        .map(str::to_string)
}
