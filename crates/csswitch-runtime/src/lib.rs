//! CSSwitch runtime: proxy lifecycle, science sandbox, profile management.
//!
//! Platform-agnostic core extracted from the Tauri desktop app.
//! The CLI binary provides concrete implementations of runtime context.

pub mod operation;
pub mod provider;
pub mod proxy;
pub mod proxy_lifecycle;
pub mod science;
pub mod settings;
pub mod system;


pub use csswitch_config;

use std::path::PathBuf;
use std::process::Child;

/// Abstract interface for platform-specific operations.
/// The CLI binary implements this; the Tauri app has its own version.
pub trait RuntimeContext {
    fn asset_root(&self) -> Option<PathBuf>;
    fn repo_root(&self) -> Option<PathBuf>;
    fn log_dir(&self) -> PathBuf;
    fn open_browser(&self, url: &str) -> Result<(), String>;
    fn append_operation_log(&self, line: &str);
}

/// Shared mutable state for managing proxy and sandbox processes.
#[derive(Default)]
pub struct AppState {
    pub proxy: Option<Child>,
    pub proxy_port: u16,
    pub secret: String,
    pub provider: String,
    pub gateway_kind: String,
    pub shim_mode: String,
    pub launch_id: String,
    pub key_fp: u64,
    pub sandbox: Option<Child>,
    pub sandbox_port: u16,
    pub sandbox_url: Option<String>,
    pub boot_error: Option<String>,
}

impl AppState {
    pub fn clear_proxy_identity(&mut self) {
        self.secret.clear();
        self.provider.clear();
        self.gateway_kind.clear();
        self.shim_mode.clear();
        self.launch_id.clear();
        self.key_fp = 0;
    }

    pub fn stop_proxy(&mut self) {
        system::kill_child(&mut self.proxy);
        self.clear_proxy_identity();
    }

    pub fn stop_sandbox(&mut self) {
        system::kill_child(&mut self.sandbox);
        self.sandbox_url = None;
    }
}

pub fn kill_child(slot: &mut Option<Child>) {
    system::kill_child(slot);
}

pub fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
