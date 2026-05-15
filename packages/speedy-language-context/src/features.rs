//! Feature toggles persisted in `.speedy/config.toml` under `[features]`.

use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Features {
    #[serde(default = "default_true")]
    pub speedy_indexer: bool,
    #[serde(default = "default_true")]
    pub language_context: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Features {
    fn default() -> Self {
        Self {
            speedy_indexer: true,
            language_context: true,
        }
    }
}

impl Features {
    /// Read `.speedy/config.toml`, look for `[features]`. Missing file or
    /// missing section → returns `Default::default()` rather than erroring;
    /// the user might just not have configured anything yet.
    pub fn load(workspace_root: &Path) -> Self {
        let path = workspace_root.join(".speedy").join("config.toml");
        if !path.exists() {
            return Features::default();
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return Features::default(),
        };
        // Parse loosely so that other unrelated keys in config.toml don't
        // trip us up.
        let parsed: toml::Value = match toml::from_str(&raw) {
            Ok(v) => v,
            Err(_) => return Features::default(),
        };
        if let Some(section) = parsed.get("features") {
            if let Ok(f) = section.clone().try_into::<Features>() {
                return f;
            }
        }
        Features::default()
    }

    /// Persist this struct into `.speedy/config.toml`'s `[features]` section,
    /// preserving any other top-level tables that were already there.
    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let dir = workspace_root.join(".speedy");
        std::fs::create_dir_all(&dir).context("creating .speedy/")?;
        let path = dir.join("config.toml");

        // Read+merge to avoid stomping on neighbouring sections.
        let mut doc: toml::Value = if path.exists() {
            let raw = std::fs::read_to_string(&path).unwrap_or_default();
            toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::value::Table::new()))
        } else {
            toml::Value::Table(toml::value::Table::new())
        };

        let features_value = toml::Value::try_from(self).context("serializing features")?;
        if let toml::Value::Table(table) = &mut doc {
            table.insert("features".to_string(), features_value);
        }

        let serialized = toml::to_string_pretty(&doc).context("serializing config.toml")?;
        std::fs::write(&path, serialized).context("writing config.toml")?;
        Ok(())
    }
}
