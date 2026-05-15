//! Thin IPC client that talks to the centralised Speedy daemon to notify it
//! about file changes and feature status.

use anyhow::{Context, Result};

use crate::features::Features;
use speedy_core::daemon_client::DaemonClient;

pub struct DaemonBridge {
    client: DaemonClient,
}

impl DaemonBridge {
    pub fn new(socket_name: &str) -> Self {
        Self {
            client: DaemonClient::new(socket_name.to_string()),
        }
    }

    /// Tell the daemon that `files` inside `workspace` were touched.
    /// Format on the wire: `slc-notify\t<workspace>\t<f1>\t<f2>...\n`
    pub async fn notify_files_changed(&self, workspace: &str, files: &[&str]) -> Result<()> {
        let mut payload = String::from("slc-notify\t");
        payload.push_str(workspace);
        for f in files {
            payload.push('\t');
            payload.push_str(f);
        }
        let resp = self
            .client
            .cmd_raw(&payload)
            .await
            .context("daemon IPC failed")?;
        if resp.trim() == "ok" {
            Ok(())
        } else {
            Err(anyhow::anyhow!("daemon replied: {resp}"))
        }
    }

    /// Ask the daemon for the global feature toggles.
    pub async fn get_feature_status(&self) -> Result<Features> {
        let resp = self
            .client
            .cmd_raw("feature-status")
            .await
            .context("daemon IPC failed")?;
        let f: Features = serde_json::from_str(resp.trim()).context("parsing feature-status JSON")?;
        Ok(f)
    }
}
