use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

// ── helpers ───────────────────────────────────────────────────────────────────

fn bin() -> PathBuf {
    let suffix = std::env::consts::EXE_SUFFIX;
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("debug")
        .join(format!("speedy-text-context{suffix}"))
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn temp_workspace() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("speedy_text_test_{}_{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Creates a workspace with predictable Dummy occurrences:
///
/// docs/guide.md (4 occurrences of "Dummy"):
///   line 1: "The Dummy symbol"        → isolated
///   line 2: "foo Dummy bar"           → isolated
///   line 3: "$Dummy$"                 → isolated_special
///   line 4: "PREFIX_Dummy_SUFFIX"     → sub-token isolated_special (whole token is isolated)
///
/// src/lib.rs (2 occurrences of "Dummy" / "dummy"):
///   line 1: "// Dummy"                → isolated
///   line 2: "// dummy is here"        → isolated  (only found with --ignore-case)
fn make_workspace() -> PathBuf {
    let dir = temp_workspace();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join(".speedyextensions"), "md\nrs\n").unwrap();
    std::fs::write(
        dir.join("docs").join("guide.md"),
        "The Dummy symbol\nfoo Dummy bar\n$Dummy$\nPREFIX_Dummy_SUFFIX\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("lib.rs"),
        "// Dummy\n// dummy is here\npub fn helper() -> bool { true }\n",
    )
    .unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run speedy-text-context");
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (code, stdout, stderr)
}

fn index(dir: &Path) {
    let (code, _, stderr) = run(dir, &["index", "."]);
    assert_eq!(code, 0, "index failed: {stderr}");
}

fn query_json(dir: &Path, extra_args: &[&str]) -> serde_json::Value {
    let mut args = vec!["query", "."];
    args.extend_from_slice(extra_args);
    let (code, stdout, stderr) = run(dir, &args);
    assert_eq!(code, 0, "query failed: {stderr}");
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n---\n{stdout}"))
}

fn count(v: &serde_json::Value) -> u64 {
    v["count"].as_u64().unwrap_or_else(|| panic!("missing 'count' in: {v}"))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_index_creates_db() {
    let dir = make_workspace();
    index(&dir);
    assert!(
        dir.join(".speedy-text").join("index.db").exists(),
        ".speedy-text/index.db not created"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_status_after_index() {
    let dir = make_workspace();
    index(&dir);
    let (code, stdout, stderr) = run(&dir, &["status", "."]);
    assert_eq!(code, 0, "status failed: {stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("status must be JSON");
    assert_eq!(v["files"].as_i64(), Some(2), "expected 2 indexed files: {v}");
    assert!(v["occurrences"].as_i64().unwrap_or(0) > 0, "no occurrences: {v}");
    assert!(v["unique_symbols"].as_i64().unwrap_or(0) > 0, "no symbols: {v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_query_default_cased_finds_all_dummy() {
    let dir = make_workspace();
    index(&dir);
    // cased (default) expands to isolated_special + isolated
    // guide.md: Dummy(isolated)×2, Dummy(isolated_special)×1, Dummy sub-token(isolated_special)×1
    // lib.rs:  Dummy(isolated)×1
    // Total: 5
    let v = query_json(&dir, &["Dummy"]);
    assert_eq!(count(&v), 5, "expected 5 cased results: {v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_query_isolated_finds_space_separated_only() {
    let dir = make_workspace();
    index(&dir);
    // isolated only:
    // guide.md: "The Dummy symbol"(1), "foo Dummy bar"(1) = 2
    // lib.rs:   "// Dummy"(1) = 1
    // Total: 3  (not $Dummy$ isolated_special, not PREFIX_Dummy_SUFFIX sub-token isolated_special)
    let v = query_json(&dir, &["Dummy", "--type", "isolated"]);
    assert_eq!(count(&v), 3, "expected 3 isolated results: {v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_query_isolated_special_includes_non_alnum_delimited() {
    let dir = make_workspace();
    index(&dir);
    // isolated_special expands to: isolated_special + isolated
    // isolated:        Dummy×2 in md + Dummy×1 in rs = 3
    // isolated_special: $Dummy$×1 + PREFIX_Dummy_SUFFIX sub-token×1 = 2
    // Total: 5
    let v = query_json(&dir, &["Dummy", "--type", "isolated_special"]);
    assert_eq!(count(&v), 5, "expected 5 isolated_special results: {v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_query_ext_filter_md_only() {
    let dir = make_workspace();
    index(&dir);
    // Only md: 4 (2 isolated + 1 isolated_special + 1 sub-token isolated_special)
    let v = query_json(&dir, &["Dummy", "--ext", "md"]);
    assert_eq!(count(&v), 4, "expected 4 results in .md files: {v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_query_ext_filter_rs_only() {
    let dir = make_workspace();
    index(&dir);
    // Only rs: "// Dummy" line = 1 result
    let v = query_json(&dir, &["Dummy", "--ext", "rs"]);
    assert_eq!(count(&v), 1, "expected 1 result in .rs files: {v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_query_ignore_case_finds_lowercase_too() {
    let dir = make_workspace();
    index(&dir);
    // "Dummy" appears 5×; "dummy" appears 1× in lib.rs line 2
    let v = query_json(&dir, &["dummy", "--ignore-case"]);
    assert_eq!(count(&v), 6, "expected 6 ignore-case results: {v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_query_no_results_returns_count_zero() {
    let dir = make_workspace();
    index(&dir);
    let v = query_json(&dir, &["NonExistentSymbolXYZ123"]);
    assert_eq!(count(&v), 0, "expected count 0 for missing symbol: {v}");
    assert!(v["results"].as_array().map(|a| a.is_empty()).unwrap_or(false));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_query_result_fields() {
    let dir = make_workspace();
    index(&dir);
    let v = query_json(&dir, &["Dummy", "--type", "isolated", "--ext", "rs"]);
    // "// Dummy" in lib.rs line 1: col_start=3 ("// " is 3 bytes), col_end=8
    let results = v["results"].as_array().expect("results must be array");
    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert!(r["file"].as_str().unwrap_or("").contains("lib.rs"), "file: {r}");
    assert_eq!(r["line"].as_u64(), Some(1), "line_no: {r}");
    assert_eq!(r["col_start"].as_u64(), Some(3), "col_start: {r}");
    assert_eq!(r["col_end"].as_u64(), Some(8), "col_end: {r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_sync_detects_modified_file() {
    let dir = make_workspace();
    index(&dir);

    // Before modification: 4 results in md
    let before = query_json(&dir, &["Dummy", "--ext", "md"]);
    assert_eq!(count(&before), 4);

    // Add one more Dummy to guide.md
    let md_path = dir.join("docs").join("guide.md");
    let mut content = std::fs::read_to_string(&md_path).unwrap();
    content.push_str("Another Dummy here\n");
    std::fs::write(&md_path, &content).unwrap();

    let (code, _, stderr) = run(&dir, &["sync", "."]);
    assert_eq!(code, 0, "sync failed: {stderr}");

    let after = query_json(&dir, &["Dummy", "--ext", "md"]);
    assert_eq!(count(&after), 5, "expected 5 after adding Dummy: {after}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_sync_removes_deleted_file() {
    let dir = make_workspace();
    index(&dir);

    // lib.rs contributes 1 Dummy; delete it
    std::fs::remove_file(dir.join("src").join("lib.rs")).unwrap();

    let (code, _, stderr) = run(&dir, &["sync", "."]);
    assert_eq!(code, 0, "sync failed: {stderr}");

    // rs file results should be gone
    let v = query_json(&dir, &["Dummy", "--ext", "rs"]);
    assert_eq!(count(&v), 0, "expected 0 rs results after deleting lib.rs: {v}");

    // md results still intact
    let v_md = query_json(&dir, &["Dummy", "--ext", "md"]);
    assert_eq!(count(&v_md), 4, "expected 4 md results to remain: {v_md}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_sync_unchanged_file_skipped() {
    let dir = make_workspace();
    index(&dir);

    // Capture status before sync
    let before: serde_json::Value = {
        let (_, out, _) = run(&dir, &["status", "."]);
        serde_json::from_str(&out).unwrap()
    };

    // Sync without any changes
    let (code, _, stderr) = run(&dir, &["sync", "."]);
    assert_eq!(code, 0, "sync failed: {stderr}");
    assert!(stderr.contains("0 updated"), "expected 0 updated in: {stderr}");

    // Counts must be identical
    let after: serde_json::Value = {
        let (_, out, _) = run(&dir, &["status", "."]);
        serde_json::from_str(&out).unwrap()
    };
    assert_eq!(before["occurrences"], after["occurrences"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_index_does_not_index_speedy_text_dir() {
    let dir = make_workspace();
    index(&dir);

    // Write a file with a unique symbol inside .speedy-text/ (where the DB lives)
    let internal = dir.join(".speedy-text").join("internal.txt");
    std::fs::write(&internal, "UniqueInternalSymbol123\n").unwrap();

    // Re-index (index always clears first)
    index(&dir);

    let v = query_json(&dir, &["UniqueInternalSymbol123"]);
    assert_eq!(count(&v), 0, ".speedy-text/ must not be indexed: {v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_index_is_idempotent() {
    let dir = make_workspace();
    index(&dir);
    let c1 = count(&query_json(&dir, &["Dummy"]));
    // Re-index
    index(&dir);
    let c2 = count(&query_json(&dir, &["Dummy"]));
    assert_eq!(c1, c2, "double index must produce same results");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_sub_token_col_positions() {
    let dir = make_workspace();
    index(&dir);
    // "PREFIX_Dummy_SUFFIX" → Dummy sub-token at col 7..12
    let v = query_json(&dir, &["Dummy", "--type", "isolated_special", "--ext", "md"]);
    let results = v["results"].as_array().expect("array");
    let sub = results
        .iter()
        .find(|r| r["line"].as_u64() == Some(4))
        .expect("should have a result on line 4 (PREFIX_Dummy_SUFFIX)");
    assert_eq!(sub["col_start"].as_u64(), Some(7), "col_start of sub-token: {sub}");
    assert_eq!(sub["col_end"].as_u64(), Some(12), "col_end of sub-token: {sub}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_cased_finds_camel_sub_token() {
    let dir = make_workspace();
    // Add a file with a camelCase token containing Dummy
    std::fs::write(dir.join("docs").join("camel.md"), "FooDummyBar\n").unwrap();
    index(&dir);

    // --type cased must find Dummy inside FooDummyBar
    let v = query_json(&dir, &["Dummy", "--type", "cased", "--ext", "md"]);
    let results = v["results"].as_array().expect("results must be array");
    assert!(
        results.iter().any(|r| r["file"].as_str().unwrap_or("").contains("camel.md")),
        "cased query must find Dummy inside FooDummyBar: {v}"
    );

    // --type isolated_special must NOT find Dummy in camel.md
    // (FooDummyBar has no _ / - separators; the only stored entry is the whole token)
    let v_iso = query_json(&dir, &["Dummy", "--type", "isolated_special", "--ext", "md"]);
    let iso_results = v_iso["results"].as_array().expect("array");
    assert!(
        !iso_results.iter().any(|r| r["file"].as_str().unwrap_or("").contains("camel.md")),
        "isolated_special must not find Dummy as camel sub-token: {v_iso}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── .speedyextensions tests ───────────────────────────────────────────────────

#[test]
fn test_speedyextensions_created_on_first_index() {
    let dir = temp_workspace(); // no .speedyextensions pre-created
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs").join("guide.md"), "Hello\n").unwrap();
    assert!(!dir.join(".speedyextensions").exists(), "should not exist before index");
    index(&dir);
    assert!(dir.join(".speedyextensions").exists(), ".speedyextensions must be created by index");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_speedyextensions_default_includes_md_and_rs() {
    let dir = make_workspace();
    index(&dir);
    // Both .md and .rs must be indexed (they're in the default list)
    assert!(count(&query_json(&dir, &["Dummy", "--ext", "md"])) > 0, "md must be indexed");
    assert!(count(&query_json(&dir, &["Dummy", "--ext", "rs"])) > 0, "rs must be indexed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_speedyextensions_restricts_indexed_extensions() {
    let dir = make_workspace();
    // Only index .md — lib.rs should NOT be indexed
    std::fs::write(dir.join(".speedyextensions"), "md\n").unwrap();
    index(&dir);
    assert_eq!(count(&query_json(&dir, &["Dummy", "--ext", "rs"])), 0, "rs must be excluded");
    assert!(count(&query_json(&dir, &["Dummy", "--ext", "md"])) > 0, "md must be included");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_speedyextensions_supports_dot_prefix_and_comments() {
    let dir = make_workspace();
    // Dot-prefixed extensions and comments should both be handled correctly
    std::fs::write(
        dir.join(".speedyextensions"),
        "# only markdown\n.md\n# end\n",
    ).unwrap();
    index(&dir);
    let (_, out, _) = run(&dir, &["status", "."]);
    let status: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(status["files"].as_i64(), Some(1), "only guide.md should be indexed: {status}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_speedyextensions_not_overwritten_if_exists() {
    let dir = make_workspace();
    let custom = "# custom\ntxt\n";
    std::fs::write(dir.join(".speedyextensions"), custom).unwrap();
    index(&dir);
    let read_back = std::fs::read_to_string(dir.join(".speedyextensions")).unwrap();
    assert_eq!(read_back, custom, ".speedyextensions must not be overwritten");
    let _ = std::fs::remove_dir_all(&dir);
}
