use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

const SPEEDY_HOOK_MARKER: &str = "# Speedy — managed hook";

const TEMPLATES: &[(&str, &str)] = &[
    ("post-commit",   include_str!("../../../scripts/git-hooks/post-commit.tpl")),
    ("post-checkout", include_str!("../../../scripts/git-hooks/post-checkout.tpl")),
    ("post-merge",    include_str!("../../../scripts/git-hooks/post-merge.tpl")),
    ("post-rewrite",  include_str!("../../../scripts/git-hooks/post-rewrite.tpl")),
];

const HOOK_NAMES: &[&str] = &["post-commit", "post-checkout", "post-merge", "post-rewrite"];

pub struct InstallReport {
    pub installed: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

/// Resolve the speedy executable to embed in hook scripts.
/// Tries `current_exe()` first, then falls back to a PATH search.
pub fn resolve_speedy_exe() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(canonical) = exe.canonicalize() {
            return Ok(canonical);
        }
        // canonicalize failed (e.g. symlink target missing) but the raw path exists
        if exe.exists() {
            return Ok(exe);
        }
    }
    search_path("speedy").context(
        "speedy executable not found via current_exe or PATH — \
         make sure speedy is installed and try again",
    )
}

/// Resolve the `speedy-language-context` executable. Returns `None` if not found;
/// the hook template already handles the missing-binary case gracefully.
pub fn resolve_slc_exe() -> Option<PathBuf> {
    // Same dir as the currently running speedy binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let name = if cfg!(windows) {
                "speedy-language-context.exe"
            } else {
                "speedy-language-context"
            };
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    search_path("speedy-language-context")
}

fn search_path(bin: &str) -> Option<PathBuf> {
    search_in_paths(std::env::var_os("PATH").as_deref()?, bin)
}

/// Separated from `search_path` so tests can pass an arbitrary PATH value.
pub(crate) fn search_in_paths(path_var: &std::ffi::OsStr, bin: &str) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        format!("{bin}.exe")
    } else {
        bin.to_string()
    };
    std::env::split_paths(path_var)
        .map(|dir| dir.join(&name))
        .find(|p| p.is_file())
}

/// Convert a native path to the POSIX form expected by Git-Bash sh scripts.
/// On non-Windows this is a no-op.
/// `C:\Users\foo\speedy.exe` → `/c/Users/foo/speedy.exe`
fn normalize_for_sh(path: &Path) -> String {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        // Strip extended-length prefix \\?\ if present
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        let s = s.replace('\\', "/");
        // C:/foo → /c/foo
        if s.len() >= 3 && s.as_bytes()[1] == b':' && s.as_bytes()[2] == b'/' {
            let drive = s.chars().next().unwrap().to_ascii_lowercase();
            return format!("/{}{}", drive, &s[2..]);
        }
        s
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().into_owned()
    }
}

/// Find the hooks directory for a given repo root, respecting `core.hooksPath`.
pub fn resolve_hooks_dir(repo_root: &Path) -> Result<PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(repo_root)
        .output()
        .context("failed to run `git rev-parse --git-path hooks` — is git installed?")?;

    if !out.status.success() {
        anyhow::bail!(
            "{} is not inside a git repository",
            repo_root.display()
        );
    }

    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let hooks = if Path::new(&raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        repo_root.join(raw)
    };

    Ok(hooks)
}

/// Install Speedy-managed git hooks into the repo at `repo_root`.
/// Set `force = true` to overwrite non-Speedy hooks without prompting.
pub fn install_hooks(repo_root: &Path, force: bool) -> Result<InstallReport> {
    let exe = resolve_speedy_exe()?;
    let exe_str = normalize_for_sh(&exe);

    // SLC is optional — empty string → the hook's PATH fallback handles it.
    let slc_str = resolve_slc_exe()
        .as_deref()
        .map(normalize_for_sh)
        .unwrap_or_default();

    let hooks_dir = resolve_hooks_dir(repo_root)?;
    std::fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("creating hooks dir {}", hooks_dir.display()))?;

    let mut report = InstallReport {
        installed: vec![],
        skipped: vec![],
    };

    for (hook_name, template) in TEMPLATES {
        let hook_path = hooks_dir.join(hook_name);

        if hook_path.exists() && !force {
            let existing = std::fs::read_to_string(&hook_path).unwrap_or_default();
            if !existing.contains(SPEEDY_HOOK_MARKER) {
                report.skipped.push((
                    hook_name.to_string(),
                    "existing non-Speedy hook found — use --force to overwrite".to_string(),
                ));
                continue;
            }
        }

        let script = template
            .replace("{{SPEEDY_EXE}}", &exe_str)
            .replace("{{SLC_EXE}}", &slc_str);
        std::fs::write(&hook_path, &script)
            .with_context(|| format!("writing {}", hook_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))
                .with_context(|| format!("chmod +x {}", hook_path.display()))?;
        }

        report.installed.push(hook_name.to_string());
    }

    Ok(report)
}

/// Remove only Speedy-managed hooks (identified by the marker comment).
pub fn uninstall_hooks(repo_root: &Path) -> Result<Vec<String>> {
    let hooks_dir = resolve_hooks_dir(repo_root)?;
    let mut removed = vec![];

    for name in HOOK_NAMES {
        let hook_path = hooks_dir.join(name);
        if !hook_path.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&hook_path).unwrap_or_default();
        if content.contains(SPEEDY_HOOK_MARKER) {
            std::fs::remove_file(&hook_path)
                .with_context(|| format!("removing {}", hook_path.display()))?;
            removed.push(name.to_string());
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Create a temp dir with `git init` and return its path.
    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .expect("git init");
        // Minimal git config so git doesn't complain on Windows CI
        std::process::Command::new("git")
            .args(["config", "user.email", "test@speedy"])
            .current_dir(dir.path())
            .status()
            .ok();
        std::process::Command::new("git")
            .args(["config", "user.name", "Speedy Test"])
            .current_dir(dir.path())
            .status()
            .ok();
        dir
    }

    fn hooks_dir(repo: &tempfile::TempDir) -> PathBuf {
        repo.path().join(".git").join("hooks")
    }

    fn fake_exe() -> PathBuf {
        std::env::current_exe().unwrap()
    }

    // ── normalize_for_sh ─────────────────────────────────────────────────────

    #[test]
    #[cfg(not(windows))]
    fn test_normalize_unix_path_unchanged() {
        assert_eq!(
            normalize_for_sh(Path::new("/usr/local/bin/speedy")),
            "/usr/local/bin/speedy"
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_normalize_windows_drive_letter() {
        assert_eq!(
            normalize_for_sh(Path::new(r"C:\Users\foo\speedy.exe")),
            "/c/Users/foo/speedy.exe"
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_normalize_strips_unc_prefix() {
        assert_eq!(
            normalize_for_sh(Path::new(r"\\?\C:\Users\foo\speedy.exe")),
            "/c/Users/foo/speedy.exe"
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_normalize_uppercase_drive() {
        assert_eq!(
            normalize_for_sh(Path::new(r"D:\tools\speedy.exe")),
            "/d/tools/speedy.exe"
        );
    }

    // ── template placeholder ─────────────────────────────────────────────────

    #[test]
    fn test_placeholder_replaced() {
        let tpl = "SPEEDY=\"{{SPEEDY_EXE}}\"\n";
        assert_eq!(
            tpl.replace("{{SPEEDY_EXE}}", "/c/bin/speedy"),
            "SPEEDY=\"/c/bin/speedy\"\n"
        );
    }

    #[test]
    fn test_all_templates_have_marker() {
        for (name, tpl) in TEMPLATES {
            assert!(
                tpl.contains(SPEEDY_HOOK_MARKER),
                "template {name} is missing the managed-hook marker"
            );
        }
    }

    #[test]
    fn test_all_templates_have_placeholder() {
        for (name, tpl) in TEMPLATES {
            assert!(
                tpl.contains("{{SPEEDY_EXE}}"),
                "template {name} is missing the {{SPEEDY_EXE}} placeholder"
            );
        }
    }

    #[test]
    fn test_all_templates_have_slc_placeholder() {
        for (name, tpl) in TEMPLATES {
            assert!(
                tpl.contains("{{SLC_EXE}}"),
                "template {name} is missing the {{SLC_EXE}} placeholder"
            );
        }
    }

    #[test]
    fn test_all_templates_have_fallback_to_path() {
        for (name, tpl) in TEMPLATES {
            assert!(
                tpl.contains("command -v speedy"),
                "template {name} is missing the PATH fallback"
            );
        }
    }

    // ── resolve_hooks_dir ────────────────────────────────────────────────────

    #[test]
    fn test_resolve_hooks_dir_default() {
        let repo = git_repo();
        let dir = resolve_hooks_dir(repo.path()).unwrap();
        assert_eq!(dir, hooks_dir(&repo));
    }

    #[test]
    fn test_resolve_hooks_dir_not_a_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let result = resolve_hooks_dir(tmp.path());
        assert!(result.is_err(), "expected error for non-repo dir");
    }

    // ── install_hooks ────────────────────────────────────────────────────────

    #[test]
    fn test_install_creates_all_hooks() {
        let repo = git_repo();
        let exe = fake_exe();
        let report = install_hooks_with_exe(repo.path(), false, &exe).unwrap();

        assert_eq!(report.installed.len(), HOOK_NAMES.len());
        assert!(report.skipped.is_empty());

        for name in HOOK_NAMES {
            let path = hooks_dir(&repo).join(name);
            assert!(path.exists(), "{name} hook not written");
        }
    }

    #[test]
    fn test_install_hook_contains_marker() {
        let repo = git_repo();
        let exe = fake_exe();
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();

        let content = fs::read_to_string(hooks_dir(&repo).join("post-commit")).unwrap();
        assert!(content.contains(SPEEDY_HOOK_MARKER));
    }

    #[test]
    fn test_install_hook_contains_exe_path() {
        let repo = git_repo();
        let exe = fake_exe();
        let exe_str = normalize_for_sh(&exe);
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();

        let content = fs::read_to_string(hooks_dir(&repo).join("post-commit")).unwrap();
        assert!(
            content.contains(&exe_str),
            "hook does not contain exe path '{exe_str}'"
        );
    }

    #[test]
    fn test_install_no_placeholder_left() {
        let repo = git_repo();
        let exe = fake_exe();
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();

        for name in HOOK_NAMES {
            let content = fs::read_to_string(hooks_dir(&repo).join(name)).unwrap();
            assert!(
                !content.contains("{{SPEEDY_EXE}}"),
                "{name}: placeholder not replaced"
            );
        }
    }

    #[test]
    fn test_install_skips_existing_non_speedy_hook_without_force() {
        let repo = git_repo();
        fs::create_dir_all(hooks_dir(&repo)).unwrap();
        fs::write(hooks_dir(&repo).join("post-commit"), "#!/bin/sh\necho hi\n").unwrap();

        let exe = fake_exe();
        let report = install_hooks_with_exe(repo.path(), false, &exe).unwrap();

        assert!(
            report.skipped.iter().any(|(n, _)| n == "post-commit"),
            "post-commit should be in skipped"
        );
        // The other hooks are still installed
        assert_eq!(report.installed.len(), HOOK_NAMES.len() - 1);
    }

    #[test]
    fn test_install_force_overwrites_non_speedy_hook() {
        let repo = git_repo();
        fs::create_dir_all(hooks_dir(&repo)).unwrap();
        fs::write(
            hooks_dir(&repo).join("post-commit"),
            "#!/bin/sh\necho hi\n",
        )
        .unwrap();

        let exe = fake_exe();
        let report = install_hooks_with_exe(repo.path(), true, &exe).unwrap();

        assert!(report.skipped.is_empty());
        assert_eq!(report.installed.len(), HOOK_NAMES.len());

        let content = fs::read_to_string(hooks_dir(&repo).join("post-commit")).unwrap();
        assert!(content.contains(SPEEDY_HOOK_MARKER));
    }

    #[test]
    fn test_install_overwrites_existing_speedy_hook_without_force() {
        let repo = git_repo();
        let exe = fake_exe();
        // Install once
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();
        // Install again — should overwrite without needing force
        let report = install_hooks_with_exe(repo.path(), false, &exe).unwrap();

        assert_eq!(report.installed.len(), HOOK_NAMES.len());
        assert!(report.skipped.is_empty());
    }

    // ── uninstall_hooks ──────────────────────────────────────────────────────

    #[test]
    fn test_uninstall_removes_speedy_hooks() {
        let repo = git_repo();
        let exe = fake_exe();
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();

        let removed = uninstall_hooks(repo.path()).unwrap();
        assert_eq!(removed.len(), HOOK_NAMES.len());

        for name in HOOK_NAMES {
            assert!(
                !hooks_dir(&repo).join(name).exists(),
                "{name} should have been deleted"
            );
        }
    }

    #[test]
    fn test_uninstall_preserves_non_speedy_hooks() {
        let repo = git_repo();
        fs::create_dir_all(hooks_dir(&repo)).unwrap();
        // Write a non-speedy hook
        fs::write(
            hooks_dir(&repo).join("post-commit"),
            "#!/bin/sh\necho manual\n",
        )
        .unwrap();

        let removed = uninstall_hooks(repo.path()).unwrap();

        assert!(removed.is_empty(), "should not have removed non-speedy hook");
        assert!(
            hooks_dir(&repo).join("post-commit").exists(),
            "non-speedy hook should be untouched"
        );
    }

    #[test]
    fn test_uninstall_returns_empty_when_no_hooks() {
        let repo = git_repo();
        let removed = uninstall_hooks(repo.path()).unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn test_install_then_uninstall_roundtrip() {
        let repo = git_repo();
        let exe = fake_exe();
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();
        let removed = uninstall_hooks(repo.path()).unwrap();
        assert_eq!(removed.len(), HOOK_NAMES.len());

        // Second uninstall is a no-op
        let removed2 = uninstall_hooks(repo.path()).unwrap();
        assert!(removed2.is_empty());
    }

    // ── template content: structure ──────────────────────────────────────────

    #[test]
    fn test_all_templates_start_with_shebang() {
        for (name, tpl) in TEMPLATES {
            assert!(
                tpl.starts_with("#!/bin/sh"),
                "{name} does not start with #!/bin/sh"
            );
        }
    }

    #[test]
    fn test_all_templates_end_with_exit_0() {
        for (name, tpl) in TEMPLATES {
            assert!(
                tpl.trim_end().ends_with("exit 0"),
                "{name} does not end with 'exit 0'"
            );
        }
    }

    #[test]
    fn test_all_templates_have_skip_hooks_guard() {
        for (name, tpl) in TEMPLATES {
            assert!(
                tpl.contains("SPEEDY_SKIP_HOOKS"),
                "{name} is missing SPEEDY_SKIP_HOOKS guard"
            );
        }
    }

    #[test]
    fn test_all_templates_have_exe_not_found_guard() {
        // If neither the hardcoded path nor PATH has speedy, the hook must exit silently.
        for (name, tpl) in TEMPLATES {
            assert!(
                tpl.contains(r#"[ -n "$SPEEDY" ] || exit 0"#),
                "{name} is missing the 'not found → exit 0' guard"
            );
        }
    }

    #[test]
    fn test_hook_names_and_templates_are_aligned() {
        // Every name in HOOK_NAMES must appear in TEMPLATES (same set, same order).
        assert_eq!(HOOK_NAMES.len(), TEMPLATES.len(), "count mismatch");
        for ((tpl_name, _), hook_name) in TEMPLATES.iter().zip(HOOK_NAMES.iter()) {
            assert_eq!(tpl_name, hook_name, "order mismatch");
        }
    }

    // ── template content: per-hook semantics ─────────────────────────────────

    fn tpl(name: &str) -> &'static str {
        TEMPLATES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| *t)
            .unwrap_or_else(|| panic!("template '{name}' not found"))
    }

    #[test]
    fn test_post_commit_uses_git_diff_tree() {
        assert!(tpl("post-commit").contains("git diff-tree"));
    }

    #[test]
    fn test_post_commit_daemon_path_uses_exec_index() {
        // When daemon is up, post-commit should use `daemon exec -- index`
        assert!(tpl("post-commit").contains("daemon exec -- index"));
    }

    #[test]
    fn test_post_commit_nodaemon_path_uses_index() {
        // When daemon is down, post-commit should use `SPEEDY_NO_DAEMON=1 ... index`
        let t = tpl("post-commit");
        assert!(t.contains("SPEEDY_NO_DAEMON=1"));
        assert!(t.contains("index"));
    }

    #[test]
    fn test_post_checkout_has_branch_switch_guard() {
        // $3 = 0 means file checkout; hook must skip in that case
        assert!(tpl("post-checkout").contains(r#"[ "$3" = "0" ] && exit 0"#));
    }

    #[test]
    fn test_post_checkout_daemon_path_uses_sync() {
        assert!(tpl("post-checkout").contains("daemon sync"));
    }

    #[test]
    fn test_post_merge_daemon_path_uses_sync() {
        assert!(tpl("post-merge").contains("daemon sync"));
    }

    #[test]
    fn test_post_rewrite_daemon_path_uses_reindex() {
        // rebase/amend touch many files; must use `reindex`, not `sync`
        assert!(tpl("post-rewrite").contains("daemon reindex"));
    }

    #[test]
    fn test_post_rewrite_does_not_use_sync() {
        assert!(!tpl("post-rewrite").contains("daemon sync"));
    }

    #[test]
    fn test_post_merge_does_not_use_reindex() {
        assert!(!tpl("post-merge").contains("daemon reindex"));
    }

    // ── search_in_paths ───────────────────────────────────────────────────────

    #[test]
    fn test_search_in_paths_finds_exe_in_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_name = if cfg!(windows) { "speedy.exe" } else { "speedy" };
        let fake = tmp.path().join(exe_name);
        fs::write(&fake, b"").unwrap();

        let path_var = std::env::join_paths([tmp.path()]).unwrap();
        let result = search_in_paths(&path_var, "speedy");
        assert_eq!(result, Some(fake));
    }

    #[test]
    fn test_search_in_paths_returns_none_when_absent() {
        let tmp = tempfile::tempdir().unwrap(); // dir exists, but no speedy binary inside
        let path_var = std::env::join_paths([tmp.path()]).unwrap();
        assert!(search_in_paths(&path_var, "speedy").is_none());
    }

    #[test]
    fn test_search_in_paths_prefers_first_match() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let exe_name = if cfg!(windows) { "speedy.exe" } else { "speedy" };

        let first = tmp1.path().join(exe_name);
        let second = tmp2.path().join(exe_name);
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();

        let path_var = std::env::join_paths([tmp1.path(), tmp2.path()]).unwrap();
        assert_eq!(search_in_paths(&path_var, "speedy"), Some(first));
    }

    #[test]
    fn test_search_in_paths_skips_dirs_without_exe() {
        let empty = tempfile::tempdir().unwrap();
        let with_exe = tempfile::tempdir().unwrap();
        let exe_name = if cfg!(windows) { "speedy.exe" } else { "speedy" };
        let fake = with_exe.path().join(exe_name);
        fs::write(&fake, b"").unwrap();

        let path_var = std::env::join_paths([empty.path(), with_exe.path()]).unwrap();
        assert_eq!(search_in_paths(&path_var, "speedy"), Some(fake));
    }

    #[test]
    fn test_search_in_paths_finds_slc_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_name = if cfg!(windows) {
            "speedy-language-context.exe"
        } else {
            "speedy-language-context"
        };
        let fake = tmp.path().join(exe_name);
        fs::write(&fake, b"").unwrap();

        let path_var = std::env::join_paths([tmp.path()]).unwrap();
        let result = search_in_paths(&path_var, "speedy-language-context");
        assert_eq!(result, Some(fake));
    }

    // ── resolve_speedy_exe ────────────────────────────────────────────────────

    #[test]
    fn test_resolve_speedy_exe_returns_ok() {
        // In the test binary context current_exe() always succeeds.
        assert!(resolve_speedy_exe().is_ok());
    }

    #[test]
    fn test_resolve_speedy_exe_returns_existing_file() {
        let exe = resolve_speedy_exe().unwrap();
        assert!(exe.exists(), "resolved exe does not exist: {}", exe.display());
    }

    #[test]
    fn test_resolve_speedy_exe_is_absolute() {
        let exe = resolve_speedy_exe().unwrap();
        assert!(exe.is_absolute(), "resolved exe is not absolute: {}", exe.display());
    }

    // ── resolve_hooks_dir ─────────────────────────────────────────────────────

    #[test]
    fn test_resolve_hooks_dir_respects_core_hookspath() {
        let repo = git_repo();
        let custom_rel = "my-hooks";
        std::process::Command::new("git")
            .args(["config", "core.hooksPath", custom_rel])
            .current_dir(repo.path())
            .status()
            .unwrap();

        let dir = resolve_hooks_dir(repo.path()).unwrap();
        // The returned path should be relative to the repo root since we gave a relative hooksPath.
        assert_eq!(
            dir.canonicalize().unwrap_or(dir.clone()),
            repo.path().join(custom_rel).canonicalize().unwrap_or_else(|_| repo.path().join(custom_rel))
        );
    }

    // ── install edge cases ────────────────────────────────────────────────────

    #[test]
    fn test_install_creates_hooks_dir_when_missing() {
        let repo = git_repo();
        let hd = hooks_dir(&repo);
        // Remove the hooks dir so we start from scratch
        let _ = fs::remove_dir_all(&hd);
        assert!(!hd.exists(), "pre-condition: hooks dir should not exist");

        install_hooks_with_exe(repo.path(), false, &fake_exe()).unwrap();
        assert!(hd.exists(), "install_hooks should have created the hooks dir");
    }

    #[test]
    fn test_install_mixed_state_skips_non_speedy_installs_missing() {
        let repo = git_repo();
        // post-commit: non-speedy (should be skipped)
        let hd = hooks_dir(&repo);
        fs::create_dir_all(&hd).unwrap();
        fs::write(hd.join("post-commit"), "#!/bin/sh\necho manual\n").unwrap();
        // post-checkout, post-merge, post-rewrite: absent (should be installed)

        let report = install_hooks_with_exe(repo.path(), false, &fake_exe()).unwrap();

        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].0, "post-commit");
        assert_eq!(report.installed.len(), 3);
        assert!(!report.installed.contains(&"post-commit".to_string()));
    }

    #[test]
    fn test_install_is_idempotent() {
        let repo = git_repo();
        let exe = fake_exe();
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();

        let content_before = fs::read_to_string(hooks_dir(&repo).join("post-commit")).unwrap();
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();
        let content_after = fs::read_to_string(hooks_dir(&repo).join("post-commit")).unwrap();

        assert_eq!(content_before, content_after, "re-install must produce identical content");
    }

    #[test]
    fn test_installed_hook_has_no_template_artifacts() {
        // After install, the written file must contain no literal `{{` sequences.
        let repo = git_repo();
        install_hooks_with_exe(repo.path(), false, &fake_exe()).unwrap();
        for name in HOOK_NAMES {
            let content = fs::read_to_string(hooks_dir(&repo).join(name)).unwrap();
            assert!(
                !content.contains("{{"),
                "{name}: contains unreplaced template artifact"
            );
        }
    }

    #[test]
    fn test_installed_hook_contains_daemon_fallback_lines() {
        // Every installed hook must contain both the daemon-up and daemon-down branches.
        let repo = git_repo();
        install_hooks_with_exe(repo.path(), false, &fake_exe()).unwrap();
        for name in HOOK_NAMES {
            let content = fs::read_to_string(hooks_dir(&repo).join(name)).unwrap();
            assert!(content.contains("ping"), "{name}: missing daemon ping check");
            assert!(content.contains("SPEEDY_NO_DAEMON=1"), "{name}: missing SPEEDY_NO_DAEMON fallback");
        }
    }

    // ── executable bit (Unix only) ────────────────────────────────────────────

    #[test]
    #[cfg(unix)]
    fn test_installed_hook_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let repo = git_repo();
        install_hooks_with_exe(repo.path(), false, &fake_exe()).unwrap();
        for name in HOOK_NAMES {
            let meta = fs::metadata(hooks_dir(&repo).join(name)).unwrap();
            let mode = meta.permissions().mode();
            assert!(
                mode & 0o111 != 0,
                "{name}: hook is not executable (mode {mode:o})"
            );
        }
    }

    // ── uninstall partial ─────────────────────────────────────────────────────

    #[test]
    fn test_uninstall_partial_removes_only_speedy_managed() {
        let repo = git_repo();
        let exe = fake_exe();
        // Install all four
        install_hooks_with_exe(repo.path(), false, &exe).unwrap();
        // Overwrite post-commit with a non-speedy hook
        fs::write(
            hooks_dir(&repo).join("post-commit"),
            "#!/bin/sh\necho not-speedy\n",
        )
        .unwrap();

        let removed = uninstall_hooks(repo.path()).unwrap();

        // Only the three remaining speedy hooks are removed
        assert_eq!(removed.len(), 3, "expected 3 removed, got: {:?}", removed);
        assert!(!removed.contains(&"post-commit".to_string()));

        // post-commit (non-speedy) is still on disk
        assert!(hooks_dir(&repo).join("post-commit").exists());
        // The others are gone
        for name in ["post-checkout", "post-merge", "post-rewrite"] {
            assert!(!hooks_dir(&repo).join(name).exists(), "{name} should be removed");
        }
    }

    // ── helper to inject a specific exe (avoids relying on current_exe in tests) ──

    fn install_hooks_with_exe(
        repo_root: &Path,
        force: bool,
        exe: &Path,
    ) -> Result<InstallReport> {
        let exe_str = normalize_for_sh(exe);
        let hooks_dir = resolve_hooks_dir(repo_root)?;
        std::fs::create_dir_all(&hooks_dir)?;

        let mut report = InstallReport {
            installed: vec![],
            skipped: vec![],
        };

        for (hook_name, template) in TEMPLATES {
            let hook_path = hooks_dir.join(hook_name);

            if hook_path.exists() && !force {
                let existing = fs::read_to_string(&hook_path).unwrap_or_default();
                if !existing.contains(SPEEDY_HOOK_MARKER) {
                    report.skipped.push((
                        hook_name.to_string(),
                        "existing non-Speedy hook".to_string(),
                    ));
                    continue;
                }
            }

            // Replace both placeholders; SLC_EXE is optional so we use empty string.
            let script = template
                .replace("{{SPEEDY_EXE}}", &exe_str)
                .replace("{{SLC_EXE}}", "");
            fs::write(&hook_path, &script)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))?;
            }

            report.installed.push(hook_name.to_string());
        }

        Ok(report)
    }
}
