use std::path::{Path, PathBuf};
use std::process::Command;

// Packages to build with `cargo build -p <name>`
const PACKAGES: &[&str] = &[
    "speedy-ai-context",
    "speedy-daemon",
    "speedy-cli",
    "speedy-ai-context-mcp",
    "speedy-gui",
    "speedy-language-context",
    "speedy-text",
];

// Binaries to copy from target/release/ to dist/ (may differ from PACKAGES
// when one package produces multiple [[bin]] targets)
const BINARIES: &[&str] = &[
    "speedy-ai-context",
    "speedy-daemon",
    "speedy-cli",
    "speedy-ai-context-mcp",
    "speedy-gui",
    "speedy-language-context",
    "speedy-language-context-mcp",
    "speedy-text-context",
    "speedy-text-context-mcp",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().map(String::as_str);
    let clean = args.contains(&"--clean".to_string());
    let installer = args.contains(&"--installer".to_string());
    let update = args.contains(&"--update".to_string());
    let version = flag_value(&args, "--version");

    match task {
        Some("dist") => dist(clean, installer),
        Some("publish-winget") => publish_winget(version, update),
        _ => {
            eprintln!("Usage: cargo xtask <task> [flags]");
            eprintln!("Tasks:");
            eprintln!("  dist                          Incremental build + copy to dist/");
            eprintln!("  dist --clean                  Force full rebuild of all binaries");
            eprintln!("  dist --installer              Also build the Windows installer (.exe)");
            eprintln!("  dist --clean --installer      Full rebuild + installer");
            eprintln!("  publish-winget                Submit to winget (uses workspace version)");
            eprintln!("  publish-winget --update       Update existing winget package");
            eprintln!("  publish-winget --version X.Y.Z  Override version");
            std::process::exit(1);
        }
    }
}

fn dist(clean: bool, installer: bool) {
    let root = workspace_root();
    let dist = root.join("dist");
    let target = root.join("target").join("release");

    if clean {
        println!("==> Cleaning packages...");
        let packages: Vec<_> = PACKAGES.iter().flat_map(|b| ["-p", b]).collect();
        let status = Command::new("cargo")
            .arg("clean")
            .args(&packages)
            .current_dir(&root)
            .status()
            .expect("failed to run cargo clean");
        if !status.success() {
            eprintln!("cargo clean failed");
            std::process::exit(1);
        }
    }

    std::fs::create_dir_all(&dist).expect("failed to create dist/");

    println!("==> Building release binaries...");
    let packages: Vec<_> = PACKAGES.iter().flat_map(|b| ["-p", b]).collect();
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .args(&packages)
        .current_dir(&root)
        .status()
        .expect("failed to run cargo build");

    if !status.success() {
        eprintln!("cargo build --release failed");
        std::process::exit(1);
    }

    println!();
    for bin in BINARIES {
        let exe = exe_name(bin);
        let src = target.join(&exe);
        let dst = dist.join(&exe);
        if src.exists() {
            std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("copy {exe}: {e}"));
            let kb = std::fs::metadata(&dst).map(|m| m.len() / 1024).unwrap_or(0);
            println!("  dist/{exe}  ({kb} KB)");
        } else {
            eprintln!("  WARNING: {exe} not found in target/release");
        }
    }

    // Copy installer docs alongside the binaries
    for doc in &["README.txt", "INSTALLATION.md", "FOR-IA.md"] {
        let src = root.join("installer").join(doc);
        let dst = dist.join(doc);
        if src.exists() {
            std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("copy {doc}: {e}"));
            println!("  dist/{doc}");
        } else {
            eprintln!("  WARNING: installer/{doc} not found");
        }
    }

    println!("\nBinaries ready in {}", dist.display());

    if installer {
        build_installer(&root);
    }
}

fn build_installer(root: &Path) {
    println!("\n==> Building Windows installer...");
    let script = root.join("scripts").join("build-installer.ps1");
    let status = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-SkipBuild")
        .current_dir(root)
        .status()
        .expect("failed to launch build-installer.ps1");
    if !status.success() {
        eprintln!("build-installer.ps1 failed");
        std::process::exit(1);
    }
}

fn exe_name(bin: &str) -> String {
    if cfg!(windows) {
        format!("{bin}.exe")
    } else {
        bin.to_string()
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has no parent")
        .to_path_buf()
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
}

fn publish_winget(version: Option<String>, update: bool) {
    let root = workspace_root();
    let ver = version.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let script = root.join("scripts").join("submit-winget.ps1");

    println!("==> cargo publish-winget v{ver}{}",
        if update { " (--update)" } else { " (prima submission)" });

    if !script.exists() {
        eprintln!("Script non trovato: {}", script.display());
        std::process::exit(1);
    }

    let mut cmd = Command::new("powershell");
    cmd.args(["-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-Version")
        .arg(&ver)
        .current_dir(&root);

    if update {
        cmd.arg("-Update");
    }

    let status = cmd.status().expect("failed to launch submit-winget.ps1");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}
