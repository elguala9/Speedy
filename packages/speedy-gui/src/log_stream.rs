//! Background log-streaming subscription for the live log view.
//!
//! Holds a ring buffer (capped) so an aggressive `tracing` rate can't blow
//! memory. The UI thread reads the buffer once per frame; the subscription
//! task keeps appending until either the daemon closes or the handle drops.

use speedy_core::types::LogLine;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const MAX_BUFFERED: usize = 5000;

#[derive(Default)]
pub struct LogBuffer {
    pub lines: VecDeque<LogLine>,
    pub connected: bool,
    pub last_error: Option<String>,
}

impl LogBuffer {
    fn push(&mut self, line: LogLine) {
        if self.lines.len() >= MAX_BUFFERED {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
}

pub struct LogStreamHandle {
    pub buffer: Arc<Mutex<LogBuffer>>,
    pub task: Option<tokio::task::JoinHandle<()>>,
}

impl LogStreamHandle {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(LogBuffer::default())),
            task: None,
        }
    }

    pub fn start(&mut self, bridge: &crate::daemon::DaemonBridge) {
        // Idempotent: if a task is already alive, leave it.
        if self.task.as_ref().map_or(false, |t| !t.is_finished()) {
            return;
        }
        let buf = self.buffer.clone();
        let client = bridge.client();
        let task = bridge.runtime().spawn(async move {
            // Re-subscribe with backoff if the connection is refused or
            // dropped — the daemon may not be up yet when the user opens the
            // log view for the first time.
            loop {
                match client.subscribe_log().await {
                    Ok((mut rx, _handle)) => {
                        if let Ok(mut b) = buf.lock() {
                            b.connected = true;
                            b.last_error = None;
                        }
                        while let Some(line) = rx.recv().await {
                            if let Ok(mut b) = buf.lock() {
                                b.push(line);
                            }
                        }
                        if let Ok(mut b) = buf.lock() {
                            b.connected = false;
                        }
                    }
                    Err(e) => {
                        if let Ok(mut b) = buf.lock() {
                            b.connected = false;
                            b.last_error = Some(e.to_string());
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
        self.task = Some(task);
    }

    pub fn stop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        if let Ok(mut b) = self.buffer.lock() {
            b.connected = false;
        }
    }

    pub fn clear(&mut self) {
        if let Ok(mut b) = self.buffer.lock() {
            b.lines.clear();
        }
    }
}

impl Default for LogStreamHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LogStreamHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
