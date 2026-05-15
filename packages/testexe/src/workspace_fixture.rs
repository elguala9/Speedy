//! Cross-process fixture for the workspaces.json file lock. Lets integration
//! tests spawn distinct OS processes that each hammer `speedy_core::workspace`
//! to prove `fd_lock` serializes them — the in-process test only covers
//! threads of one process.
//!
//! Usage:
//!   workspace-fixture add    <path>
//!   workspace-fixture remove <path>
//!   workspace-fixture list
//!
//! Honors `SPEEDY_DAEMON_DIR` so the test driver can point all subprocesses at
//! an isolated config dir.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [op, path] if op == "add" => speedy_core::workspace::add(path).map(|_| String::from("ok")),
        [op, path] if op == "remove" => {
            speedy_core::workspace::remove(path).map(|_| String::from("ok"))
        }
        [op] if op == "list" => speedy_core::workspace::list().map(|ws| {
            let mut paths: Vec<String> = ws.into_iter().map(|e| e.path).collect();
            paths.sort();
            paths.join("\n")
        }),
        _ => {
            eprintln!("usage: workspace-fixture <add|remove> <path> | list");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(s) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
