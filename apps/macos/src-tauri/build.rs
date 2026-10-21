use std::process::Command;
fn main() {
    let root = "../../..";
    let output = Command::new("python3")
        .arg("scripts/build_identity.py")
        .current_dir(root)
        .output()
        .expect("shared build identity");
    assert!(output.status.success(), "shared build identity failed");
    let version = String::from_utf8(output.stdout).expect("UTF-8 build identity");
    println!("cargo:rustc-env=WISPNET_BUILD_ID={}", version.trim());
    let files = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .current_dir(root)
        .output()
        .expect("source file list");
    for path in String::from_utf8(files.stdout)
        .unwrap()
        .split('\0')
        .filter(|p| !p.is_empty())
    {
        println!("cargo:rerun-if-changed={root}/{path}");
    }
    for path in ["HEAD", "refs/heads/main", "index"] {
        let output = Command::new("git")
            .args(["rev-parse", "--git-path", path])
            .current_dir(root)
            .output()
            .unwrap();
        println!(
            "cargo:rerun-if-changed={root}/{}",
            String::from_utf8(output.stdout).unwrap().trim()
        );
    }
    if std::env::var_os("CARGO_FEATURE_GUI").is_some() {
        tauri_build::build();
    }
}
