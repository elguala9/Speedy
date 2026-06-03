=====================================================================
  SPEEDY — Local Semantic File System
  AI context engine for your codebase
=====================================================================

Speedy indexes your projects into a local vector database, watches for
file changes in the background, and exposes semantic search through a
CLI and two MCP servers for AI coding agents.


---------------------------------------------------------------------
  BINARIES INSTALLED
---------------------------------------------------------------------

  speedy-daemon.exe
    The single background daemon. Watches all registered workspaces,
    keeps the index up to date, and serves requests from the CLI.
    Starts automatically at login (if you selected that option).

  speedy-cli.exe
    Thin client for the daemon. Use this for scripting, indexing and
    querying from the terminal. Talks to the daemon via a local socket.

  speedy-ai-context-mcp.exe
    MCP server for semantic search. Connect this to Claude Code,
    Cursor, Windsurf, opencode, or any MCP-compatible AI agent.

  speedy-language-context-mcp.exe
    MCP server for code graph analysis. Exposes symbol skeletons,
    impact analysis, and workspace indexing for AI agents.

  speedy-text-context-mcp.exe
    MCP server for text-symbol search. Finds symbol occurrences in
    docs/config files with line/column positions for AI agents.

  speedy-gui.exe
    Desktop GUI to manage the daemon, workspaces, and view live logs.

  speedy-ai-context.exe
    The indexing/query worker. Normally called by the daemon — you
    rarely need to invoke this directly.

  speedy-language-context.exe
    The code graph daemon. Normally called by the MCP server.

  speedy-text-context.exe
    The text-symbol indexer/query worker. Normally called by the
    daemon — you rarely need to invoke this directly.


---------------------------------------------------------------------
  QUICK START
---------------------------------------------------------------------

1. The daemon starts automatically. Verify it is running:

     speedy-cli daemon status

2. Add your project to the workspace registry:

     speedy-cli workspace add C:\path\to\your\project

3. Index the project:

     speedy-cli index

4. Search your codebase:

     speedy-cli query "authentication middleware"

5. View all registered workspaces:

     speedy-cli workspace list

For a full list of commands:

     speedy-cli --help
     speedy-cli <command> --help


---------------------------------------------------------------------
  CLI COMMANDS REFERENCE
---------------------------------------------------------------------

  speedy-cli index [subdir]        Index a directory (default: .)
  speedy-cli query <text> [-k N]   Semantic search, return top N results
  speedy-cli context               Show project context summary
  speedy-cli sync                  Incremental sync of filesystem changes
  speedy-cli force [-p path]       Force a full reindex

  speedy-cli workspace add <path>  Register a workspace
  speedy-cli workspace remove <path>  Unregister a workspace
  speedy-cli workspace list        List all registered workspaces

  speedy-cli daemon status         Show daemon status
  speedy-cli daemon ping           Ping the daemon
  speedy-cli daemon stop           Gracefully stop the daemon

  Global flags:
    --json        Output in JSON format (useful for scripting)
    -p, --path    Set project root (default: current directory)


---------------------------------------------------------------------
  MCP SERVERS — CONNECT TO AI AGENTS
---------------------------------------------------------------------

Speedy ships two MCP servers. Add one or both to your AI agent config.

--- speedy-ai-context-mcp (semantic search) ---

Tools available:
  speedy_query           Semantic search in natural language
  speedy_index           Index a directory
  speedy_context         Project context summary
  speedy_workspace_add   Add a workspace to the registry
  speedy_workspace_remove  Remove a workspace
  speedy_workspace_list  List all workspaces
  speedy_force_reindex   Force a full reindex

--- speedy-language-context-mcp (code graph) ---

Tools available:
  run_pipeline       Search + impact analysis for a task
  get_skeleton       File structure at configurable detail levels
  index_status       Current index stats
  force_reindex      Force a full reindex (returns updated stats)
  workspace_add      Add a workspace
  workspace_remove   Remove a workspace
  workspace_list     List all workspaces
  save_observation   Save a note about the codebase
  search_observations  Search saved notes

--- speedy-text-context-mcp (text-symbol search) ---

Tools available:
  text_query         Find symbol occurrences in docs/config files
  text_replace       Replace symbol occurrences and re-index changed files
  text_status        Index stats (files, occurrences, unique symbols)
  text_force_reindex Drop the text index and re-index the workspace


---------------------------------------------------------------------
  MCP CONFIGURATION EXAMPLES
---------------------------------------------------------------------

--- Claude Code (claude.json / .claude/settings.json) ---

{
  "mcpServers": {
    "speedy": {
      "command": "speedy-ai-context-mcp",
      "args": []
    },
    "speedy-lc": {
      "command": "speedy-language-context-mcp",
      "args": ["--workspace", "C:\\path\\to\\your\\project"]
    },
    "speedy-text": {
      "command": "speedy-text-context-mcp",
      "args": ["--workspace", "C:\\path\\to\\your\\project"]
    }
  }
}

--- Cursor / Windsurf / opencode (mcp.json) ---

{
  "mcpServers": {
    "speedy": {
      "command": "speedy-ai-context-mcp",
      "args": [],
      "env": {
        "SPEEDY_MCP_TOP_K": "10"
      }
    }
  }
}

Note: if speedy-cli is not on PATH, add:
  "env": { "SPEEDY_BIN": "C:\\Users\\<you>\\AppData\\Local\\Programs\\Speedy\\speedy-cli.exe" }


---------------------------------------------------------------------
  ENVIRONMENT VARIABLES
---------------------------------------------------------------------

  SPEEDY_BIN              Path to speedy-cli.exe (used by MCP servers)
  SPEEDY_DEFAULT_SOCKET   Daemon socket name (default: speedy-daemon)
  SPEEDY_MCP_TOP_K        Default result count for speedy_query (default: 5)
  SPEEDY_MODEL            Embedding model name


---------------------------------------------------------------------
  DATA LOCATIONS
---------------------------------------------------------------------

  %APPDATA%\speedy\           Daemon data: workspaces.json, logs
  %USERPROFILE%\.speedy\      Global user config
  <project>\.speedy\          Per-project index (SQLite + embeddings)
  <logs dir>\*.log            Rolling daily logs for each binary


---------------------------------------------------------------------
  UNINSTALL
---------------------------------------------------------------------

Use "Add or Remove Programs" in Windows Settings, or run the
uninstaller from the Start Menu > Speedy > Uninstall Speedy.

Project indexes in <project>\.speedy\ are NOT deleted automatically.
Delete them manually if you want a clean removal.


---------------------------------------------------------------------
  MORE INFORMATION
---------------------------------------------------------------------

  GitHub:   https://github.com/elguala9/Speedy
  Issues:   https://github.com/elguala9/Speedy/issues
  Releases: https://github.com/elguala9/Speedy/releases

=====================================================================
