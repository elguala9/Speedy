use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "speedy", version, about = "Local Semantic File System")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(global = true, long, help = "Output in JSON format")]
    pub json: bool,

    #[arg(short = 'p', long = "path", help = "Project root (default: current dir)")]
    pub project_path: Option<String>,

    #[arg(short = 'r', long = "read", help = "Query the workspace with a natural language prompt")]
    pub read: Option<String>,

    #[arg(short = 'm', long = "modify", help = "Modify the workspace based on a prompt")]
    pub modify: Option<String>,

    #[arg(long = "file", help = "Target file for --modify", requires = "modify")]
    pub file: Option<String>,

    #[arg(short = 'd', long = "daemons", help = "List all workspaces tracked by the daemon")]
    pub daemons: bool,

    #[arg(long = "daemon-stop", alias = "ds", help = "Stop a daemon (use with -p <path>)")]
    pub daemon_stop: Option<String>,

    #[arg(long = "daemon-restart", alias = "dr", help = "Restart a daemon (use with -p <path>)")]
    pub daemon_restart: Option<String>,

    #[arg(long = "daemon-delete", alias = "dd", help = "Delete a daemon permanently (use with -p <path>)")]
    pub daemon_delete: Option<String>,

    #[arg(long = "daemon-create", alias = "dc", help = "Create a daemon for a workspace (use with -p <path>)")]
    pub daemon_create: Option<String>,

    #[arg(long = "daemon-status", help = "Show daemon status (use with -p <path>)")]
    pub daemon_status: Option<String>,

    #[arg(short = 'f', long = "force", help = "Force reindex of a workspace (use with -p <path>)")]
    pub force: Option<String>,

    #[arg(short = 'w', long = "workspaces", help = "List all workspaces")]
    pub workspaces: bool,

    #[arg(long = "workspace-create", alias = "wc", help = "Create a workspace and its daemon (use with -p <path>)")]
    pub workspace_create: Option<String>,

    #[arg(long = "workspace-delete", help = "Delete a workspace and its daemon (use with -p <path>)")]
    pub workspace_delete: Option<String>,

    #[arg(long = "daemon-port", help = "Daemon TCP port (default: 42137)", default_value = "42137")]
    pub daemon_port: u16,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Index a directory into the vector database")]
    Index {
        #[arg(default_value = ".")]
        subdir: String,
    },
    #[command(about = "Query the index with semantic search")]
    Query {
        query: String,
        #[arg(short = 'k', long = "top-k", default_value = "5")]
        top_k: Option<usize>,
    },
    #[command(about = "Watch a directory for changes and auto-index")]
    Watch {
        #[arg(default_value = ".")]
        subdir: String,
        #[arg(long = "detach", help = "Run watcher in background (daemon mode)")]
        detach: bool,
    },
    #[command(about = "Show project context summary")]
    Context,
    #[command(about = "Sync filesystem changes to the database incrementally")]
    Sync,
    #[command(about = "Manage the background watcher daemon (no args = start central daemon)")]
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
    #[command(about = "Manage workspaces")]
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    #[command(about = "Force reindex of a workspace")]
    Force {
        #[arg(short = 'p', help = "Workspace path (default: current dir)")]
        path: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum DaemonAction {
    #[command(about = "Register daemon to start at boot for this project")]
    Install {
        #[arg(help = "Project root (default: current dir)")]
        path: Option<String>,
    },
    #[command(about = "Unregister daemon and stop it")]
    Uninstall,
    #[command(about = "Show daemon status")]
    Status,
    #[command(about = "List all daemons")]
    List,
    #[command(about = "Stop a daemon")]
    Stop {
        #[arg(short = 'p', help = "Daemon workspace path")]
        path: String,
    },
    #[command(about = "Restart a daemon")]
    Restart {
        #[arg(short = 'p', help = "Daemon workspace path")]
        path: String,
    },
    #[command(about = "Delete a daemon permanently")]
    Delete {
        #[arg(short = 'p', help = "Daemon workspace path")]
        path: String,
    },
    #[command(about = "Create a daemon for a workspace")]
    Create {
        #[arg(short = 'p', help = "Workspace path")]
        path: String,
    },
}

#[derive(Subcommand)]
pub enum WorkspaceAction {
    #[command(about = "List all workspaces")]
    List,
    #[command(about = "Create a workspace and its daemon")]
    Create {
        #[arg(short = 'p', help = "Workspace path")]
        path: String,
    },
    #[command(about = "Delete a workspace and its daemon")]
    Delete {
        #[arg(short = 'p', help = "Workspace path")]
        path: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn test_cli_parse_index() {
        let cli = Cli::parse_from(["speedy", "index"]);
        assert!(matches!(cli.command, Some(Commands::Index { .. })));
    }

    #[test]
    fn test_cli_parse_query() {
        let cli = Cli::parse_from(["speedy", "query", "test query"]);
        assert!(matches!(cli.command, Some(Commands::Query { .. })));
    }

    #[test]
    fn test_cli_parse_watch() {
        let cli = Cli::parse_from(["speedy", "watch", "src"]);
        assert!(matches!(cli.command, Some(Commands::Watch { .. })));
    }

    #[test]
    fn test_cli_parse_watch_detach() {
        let cli = Cli::parse_from(["speedy", "watch", "--detach"]);
        assert!(matches!(cli.command, Some(Commands::Watch { subdir: _, detach: true })));
    }

    #[test]
    fn test_cli_parse_daemon_list() {
        let cli = Cli::parse_from(["speedy", "daemon", "list"]);
        assert!(matches!(cli.command, Some(Commands::Daemon { action: Some(DaemonAction::List) })));
    }

    #[test]
    fn test_cli_parse_daemon_stop() {
        let cli = Cli::parse_from(["speedy", "daemon", "stop", "-p", "/test"]);
        assert!(matches!(cli.command, Some(Commands::Daemon { action: Some(DaemonAction::Stop { .. }) })));
    }

    #[test]
    fn test_cli_parse_workspace_list() {
        let cli = Cli::parse_from(["speedy", "workspace", "list"]);
        assert!(matches!(cli.command, Some(Commands::Workspace { action: WorkspaceAction::List })));
    }

    #[test]
    fn test_cli_parse_force() {
        let cli = Cli::parse_from(["speedy", "force"]);
        assert!(matches!(cli.command, Some(Commands::Force { .. })));
    }

    #[test]
    fn test_cli_parse_daemons_flag() {
        let cli = Cli::parse_from(["speedy", "--daemons"]);
        assert!(cli.daemons);
    }

    #[test]
    fn test_cli_parse_workspaces_flag() {
        let cli = Cli::parse_from(["speedy", "--workspaces"]);
        assert!(cli.workspaces);
    }

    #[test]
    fn test_cli_parse_path_global() {
        let cli = Cli::parse_from(["speedy", "-p", "/my/proj", "index"]);
        assert_eq!(cli.project_path, Some("/my/proj".to_string()));
    }

    #[test]
    fn test_cli_parse_json_global() {
        let cli = Cli::parse_from(["speedy", "--json", "context"]);
        assert!(cli.json);
    }

    #[test]
    fn test_cli_parse_read() {
        let cli = Cli::parse_from(["speedy", "--read", "find auth code"]);
        assert_eq!(cli.read, Some("find auth code".to_string()));
    }

    #[test]
    fn test_cli_parse_modify() {
        let cli = Cli::parse_from(["speedy", "--modify", "content", "--file", "test.rs"]);
        assert_eq!(cli.modify, Some("content".to_string()));
        assert_eq!(cli.file, Some("test.rs".to_string()));
    }

    #[test]
    fn test_cli_parse_daemon_stop_top_level() {
        let cli = Cli::parse_from(["speedy", "--daemon-stop", "/test/path"]);
        assert_eq!(cli.daemon_stop, Some("/test/path".to_string()));
    }

    #[test]
    fn test_cli_parse_daemon_restart_top_level() {
        let cli = Cli::parse_from(["speedy", "--daemon-restart", "/test/path"]);
        assert_eq!(cli.daemon_restart, Some("/test/path".to_string()));
    }

    #[test]
    fn test_cli_parse_daemon_delete_top_level() {
        let cli = Cli::parse_from(["speedy", "--daemon-delete", "/test/path"]);
        assert_eq!(cli.daemon_delete, Some("/test/path".to_string()));
    }

    #[test]
    fn test_cli_parse_daemon_create_top_level() {
        let cli = Cli::parse_from(["speedy", "--daemon-create", "/test/path"]);
        assert_eq!(cli.daemon_create, Some("/test/path".to_string()));
    }

    #[test]
    fn test_cli_parse_daemon_status_top_level() {
        let cli = Cli::parse_from(["speedy", "--daemon-status", "/test/path"]);
        assert_eq!(cli.daemon_status, Some("/test/path".to_string()));
    }

    #[test]
    fn test_cli_parse_force_top_level() {
        let cli = Cli::parse_from(["speedy", "--force", "/test/path"]);
        assert_eq!(cli.force, Some("/test/path".to_string()));
    }

    #[test]
    fn test_cli_parse_workspace_create_top_level() {
        let cli = Cli::parse_from(["speedy", "--workspace-create", "/test/path"]);
        assert_eq!(cli.workspace_create, Some("/test/path".to_string()));
    }

    #[test]
    fn test_cli_parse_workspace_delete_top_level() {
        let cli = Cli::parse_from(["speedy", "--workspace-delete", "/test/path"]);
        assert_eq!(cli.workspace_delete, Some("/test/path".to_string()));
    }
}
