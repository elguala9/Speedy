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

    #[arg(short = 'w', long = "workspaces", help = "List all workspaces")]
    pub workspaces: bool,

    #[arg(long = "daemon-socket", help = "Daemon socket name")]
    pub daemon_socket: Option<String>,
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
    #[command(about = "Show project context summary")]
    Context,
    #[command(about = "Sync filesystem changes to the database incrementally")]
    Sync,
    #[command(about = "Drop and rebuild every chunk's embedding (use after changing SPEEDY_MODEL)")]
    Reembed,
    #[command(about = "Start the central background daemon")]
    Daemon,
    #[command(about = "Manage workspaces")]
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    #[command(about = "Install Speedy git hooks into the current repo")]
    InstallHooks {
        #[arg(long, help = "Repo root (default: current dir)")]
        path: Option<std::path::PathBuf>,
        #[arg(long, help = "Overwrite existing non-Speedy hooks without prompting")]
        force: bool,
    },
    #[command(about = "Remove Speedy-managed git hooks from the current repo")]
    UninstallHooks {
        #[arg(long, help = "Repo root (default: current dir)")]
        path: Option<std::path::PathBuf>,
    },
    #[command(about = "Enable a feature (speedy | slc)")]
    Enable {
        #[arg(help = "Feature name: speedy (file indexer) or slc (language context)")]
        feature: String,
    },
    #[command(about = "Disable a feature (speedy | slc)")]
    Disable {
        #[arg(help = "Feature name: speedy (file indexer) or slc (language context)")]
        feature: String,
    },
    #[command(about = "Show the status of all toggleable features")]
    Features,
}

#[derive(Subcommand)]
pub enum WorkspaceAction {
    #[command(about = "List all workspaces")]
    List,
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
    fn test_cli_parse_sync() {
        let cli = Cli::parse_from(["speedy", "sync"]);
        assert!(matches!(cli.command, Some(Commands::Sync)));
    }

    #[test]
    fn test_cli_parse_context() {
        let cli = Cli::parse_from(["speedy", "context"]);
        assert!(matches!(cli.command, Some(Commands::Context)));
    }

    #[test]
    fn test_cli_parse_daemon() {
        let cli = Cli::parse_from(["speedy", "daemon"]);
        assert!(matches!(cli.command, Some(Commands::Daemon)));
    }

    #[test]
    fn test_cli_parse_workspace_list() {
        let cli = Cli::parse_from(["speedy", "workspace", "list"]);
        assert!(matches!(cli.command, Some(Commands::Workspace { action: WorkspaceAction::List })));
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
    fn test_cli_parse_install_hooks_default() {
        let cli = Cli::parse_from(["speedy", "install-hooks"]);
        assert!(matches!(
            cli.command,
            Some(Commands::InstallHooks { path: None, force: false })
        ));
    }

    #[test]
    fn test_cli_parse_install_hooks_with_path() {
        let cli = Cli::parse_from(["speedy", "install-hooks", "--path", "/some/repo"]);
        assert!(matches!(
            cli.command,
            Some(Commands::InstallHooks { path: Some(_), force: false })
        ));
    }

    #[test]
    fn test_cli_parse_install_hooks_force() {
        let cli = Cli::parse_from(["speedy", "install-hooks", "--force"]);
        assert!(matches!(
            cli.command,
            Some(Commands::InstallHooks { force: true, .. })
        ));
    }

    #[test]
    fn test_cli_parse_uninstall_hooks_default() {
        let cli = Cli::parse_from(["speedy", "uninstall-hooks"]);
        assert!(matches!(
            cli.command,
            Some(Commands::UninstallHooks { path: None })
        ));
    }

    #[test]
    fn test_cli_parse_uninstall_hooks_with_path() {
        let cli = Cli::parse_from(["speedy", "uninstall-hooks", "--path", "/some/repo"]);
        assert!(matches!(
            cli.command,
            Some(Commands::UninstallHooks { path: Some(_) })
        ));
    }

    #[test]
    fn test_cli_parse_install_hooks_force_defaults_to_false() {
        let cli = Cli::parse_from(["speedy", "install-hooks"]);
        assert!(matches!(
            cli.command,
            Some(Commands::InstallHooks { force: false, .. })
        ));
    }

    #[test]
    fn test_cli_parse_install_hooks_force_and_path_combined() {
        let cli = Cli::parse_from(["speedy", "install-hooks", "--force", "--path", "/my/repo"]);
        assert!(matches!(
            cli.command,
            Some(Commands::InstallHooks { force: true, path: Some(_) })
        ));
    }

    #[test]
    fn test_cli_install_hooks_help_text() {
        // Verify the subcommands appear in the top-level help output.
        let cmd = Cli::command();
        let subcommand_names: Vec<_> = cmd.get_subcommands().map(|c| c.get_name()).collect();
        assert!(subcommand_names.contains(&"install-hooks"), "install-hooks missing from subcommands");
        assert!(subcommand_names.contains(&"uninstall-hooks"), "uninstall-hooks missing from subcommands");
    }

    #[test]
    fn test_cli_json_flag_works_with_install_hooks() {
        let cli = Cli::parse_from(["speedy", "--json", "install-hooks"]);
        assert!(cli.json);
        assert!(matches!(cli.command, Some(Commands::InstallHooks { .. })));
    }
}
