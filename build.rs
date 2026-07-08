use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=frontend/dist");

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
