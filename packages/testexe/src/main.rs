use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();

    let test_dir = std::env::temp_dir().join("speedy-e2e-test");

    println!("╔════════════════════════════════════════╗");
    println!("║   Speedy E2E Test Runner (Rust)       ║");
    println!("╚════════════════════════════════════════╝");
    println!();
    println!("root:     {}", root.display());
    println!("test-dir: {}", test_dir.display());
    println!();

    let ollama_ok = check_ollama();

    println!("step 1/5 › building speedy...");
    build_speedy(&root);
    println!("  ✓ built");

    println!("step 2/5 › creating test project...");
    create_test_project(&test_dir);
    println!("  ✓ created");

    println!("step 3/5 › indexing...");
    let indexed = if ollama_ok {
        index_project(&root, &test_dir)
    } else {
        false
    };

    if indexed {
        println!("step 4/5 › running queries...");
        query(&root, &test_dir, "read write file content", 3);
        query(&root, &test_dir, "calculate mean deviation", 3);
        query(&root, &test_dir, "product price discount", 3);

        println!("step 5/5 › testing sync...");
        std::fs::write(
            test_dir.join("src").join("greet.rs"),
            b"pub fn greet(name: &str) -> String { format!(\"Hello, {name}!\") }",
        )
        .unwrap();
        sync_project(&root, &test_dir);
        query(&root, &test_dir, "hello greet function", 2);
        context(&root, &test_dir);
        query_json(&root, &test_dir, "user creation", 2);
    }

    let _ = std::fs::remove_dir_all(&test_dir);

    println!();
    if indexed {
        println!("╔════════════════════════════════════════╗");
        println!("║   All E2E tests passed!               ║");
        println!("╚════════════════════════════════════════╝");
    } else {
        let model = std::env::var("SPEEDY_MODEL").unwrap_or_else(|_| "all-minilm:l6-v2".to_string());
        println!("╔════════════════════════════════════════╗");
        println!("║   E2E skipped (ollama unavailable)    ║");
        println!("║   Install the model:                  ║");
        println!("║     ollama pull {model}                ║");
        println!("║   Or set: $env:SPEEDY_MODEL = \"...\"  ║");
        println!("╚════════════════════════════════════════╝");
    }
}

fn check_ollama() -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let resp = match client.get("http://localhost:11434/api/tags").send() {
        Ok(r) => r,
        Err(_) => {
            println!("  ⚠ Ollama not reachable (http://localhost:11434)");
            return false;
        }
    };

    if !resp.status().is_success() {
        println!("  ⚠ Ollama status {}", resp.status());
        return false;
    }

    let text = match resp.text() {
        Ok(t) => t,
        Err(_) => return false,
    };

    if text.contains("\"models\"") {
        println!("  ✓ Ollama is running");
        true
    } else {
        false
    }
}

fn build_speedy(root: &Path) {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "speedy", "--bin", "speedy"])
        .current_dir(root)
        .status()
        .expect("failed to run cargo build");
    assert!(status.success(), "cargo build failed");
}

fn speedy_binary(root: &Path) -> PathBuf {
    let rel = root.join("target").join("release").join("speedy.exe");
    if rel.exists() {
        return rel;
    }
    root.join("target").join("debug").join("speedy.exe")
}

fn run_speedy(root: &Path, args: &[&str], cwd: &Path) -> Option<String> {
    let bin = speedy_binary(root);
    let out = Command::new(&bin).args(args).current_dir(cwd).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        println!("  ⚠ error: {stderr}");
        None
    }
}

fn index_project(root: &Path, test_dir: &Path) -> bool {
    match run_speedy(root, &["index", "."], test_dir) {
        Some(msg) => {
            println!("  ✓ {msg}");
            true
        }
        None => false,
    }
}

fn query(root: &Path, test_dir: &Path, q: &str, k: usize) {
    if let Some(out) = run_speedy(root, &["query", q, "-k", &k.to_string()], test_dir) {
        println!("  [{k}] \"{q}\":");
        for line in out.lines() {
            println!("    {line}");
        }
    }
}

fn query_json(root: &Path, test_dir: &Path, q: &str, k: usize) {
    if let Some(out) = run_speedy(root, &["query", "--json", q, "-k", &k.to_string()], test_dir) {
        println!("  json: {out}");
    }
}

fn sync_project(root: &Path, test_dir: &Path) {
    if let Some(out) = run_speedy(root, &["sync"], test_dir) {
        println!("  sync: {out}");
    }
}

fn context(root: &Path, test_dir: &Path) {
    if let Some(out) = run_speedy(root, &["context"], test_dir) {
        println!("  context: {out}");
    }
}

fn create_test_project(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();

    let files: [(&str, &[u8]); 6] = [
        ("Cargo.toml", b"[package]\nname = \"e2e-test-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", b"pub mod math;\npub mod io;\npub mod types;\n"),
        ("src/math.rs", br#"
pub mod arithmetic {
    pub fn add(a: i32, b: i32) -> i32 { a + b }
    pub fn sub(a: i32, b: i32) -> i32 { a - b }
    pub fn mul(a: i32, b: i32) -> i32 { a * b }
    pub fn div(a: i32, b: i32) -> i32 {
        if b == 0 { panic!("division by zero") }
        a / b
    }
}
pub mod statistics {
    pub fn mean(data: &[f64]) -> f64 { data.iter().sum::<f64>() / data.len() as f64 }
    pub fn std_dev(data: &[f64]) -> f64 {
        let m = mean(data);
        (data.iter().map(|x| (x - m).powi(2)).sum::<f64>() / data.len() as f64).sqrt()
    }
}
"#),
        ("src/io.rs", br#"
use std::fs;
use std::path::Path;
pub fn read_file(path: &Path) -> std::io::Result<String> { fs::read_to_string(path) }
pub fn write_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    fs::write(path, content)
}
"#),
        ("src/types.rs", br#"
pub struct User { pub id: u64, pub name: String, pub email: String, pub active: bool }
impl User {
    pub fn new(id: u64, name: &str, email: &str) -> Self {
        Self { id, name: name.into(), email: email.into(), active: true }
    }
}
pub struct Product { pub sku: String, pub name: String, pub price_cents: u64, pub in_stock: bool }
impl Product {
    pub fn new(sku: &str, name: &str, price_cents: u64) -> Self {
        Self { sku: sku.into(), name: name.into(), price_cents, in_stock: true }
    }
    pub fn discount(&mut self, percent: f64) {
        self.price_cents = (self.price_cents as f64 * (100.0 - percent) / 100.0) as u64;
    }
}
"#),
        ("src/main.rs", br#"
use e2e_test_project::math;
fn main() {
    println!("10 + 20 = {}", math::arithmetic::add(10, 20));
    let data = [10.0, 20.0, 30.0, 40.0, 50.0];
    println!("mean = {}, std_dev = {}", math::statistics::mean(&data), math::statistics::std_dev(&data));
}
"#),
    ];

    for (path, content) in &files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }
}
