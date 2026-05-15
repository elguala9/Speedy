pub mod dashboard;
pub mod logs;
pub mod scan;
pub mod workspaces;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Dashboard,
    Workspaces,
    Scan,
    Logs,
}

impl Default for Tab {
    fn default() -> Self {
        Self::Dashboard
    }
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Workspaces => "Workspaces",
            Self::Scan => "Scan",
            Self::Logs => "Logs",
        }
    }
}
