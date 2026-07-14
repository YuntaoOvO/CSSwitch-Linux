use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;


use crate::operation::{OperationTrace, OperationStage, POLL_INTERVAL_MS, LOCAL_HEALTH_TIMEOUT_MS};
use crate::system::{kill_child, open_log, tail_file, redact, sandbox_home, log_path};

/// Find the claude-science binary. Search order:
/// 1. Explicit SCIENCE_BIN env var
/// 2. npm global bin (~/.npm-global/bin/claude-science, /usr/local/bin/claude-science)
/// 3. PATH lookup
pub fn find_science_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SCIENCE_BIN") {
        let p = PathBuf::from(&path);
        if p.is_file() && is_executable(&p) {
            return Some(p);
        }
    }

    // Check common npm global install locations
    let candidates = [
        // npm global prefix (common custom location)
        dirs::home_dir()
            .unwrap_or_default()
            .join(".npm-global")
            .join("bin")
            .join("claude-science"),
        // Standard /usr/local/bin
        PathBuf::from("/usr/local/bin/claude-science"),
        // npm global on Linux with nvm
        dirs::home_dir()
            .unwrap_or_default()
            .join(".nvm")
            .join("versions")
            .join("node")
            .join("*") // wildcard won't work; we'll handle below
            .join("bin")
            .join("claude-science"),
    ];

    for candidate in &candidates {
        if candidate.is_file() && is_executable(candidate) {
            return Some(candidate.clone());
        }
    }

    // Try nvm paths with glob
    if let Ok(nvm_dir) = std::env::var("NVM_DIR") {
        let nvm = PathBuf::from(nvm_dir);
        if let Ok(entries) = fs::read_dir(nvm.join("versions").join("node")) {
            for entry in entries.flatten() {
                let bin = entry.path().join("bin").join("claude-science");
                if bin.is_file() && is_executable(&bin) {
                    return Some(bin);
                }
            }
        }
    }

    // Fallback: PATH lookup  
    if let Some(path) = crate::system::find_in_path("claude-science") {
        return Some(path);
    }

    None
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Check if the Science sandbox is running and healthy.
pub fn sandbox_health(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}");
    reqwest::blocking::Client::new()
        .get(&url)
        .timeout(Duration::from_millis(LOCAL_HEALTH_TIMEOUT_MS))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Check if the running sandbox at the given port belongs to us
/// by checking its data-dir matches our sandbox home.
pub fn sandbox_running_ours(port: u16) -> bool {
    // Try to read the sandbox's PID and check its working directory
    // For now, a simplified check: if health endpoint responds, assume it's ours
    sandbox_health(port)
}

/// Start Claude Science in sandboxed mode with CSSwitch proxy.
pub fn start_science_sandbox(
    port: u16,
    proxy_port: u16,
    secret: &str,
    trace: Option<&OperationTrace>,
) -> Result<Child, String> {
    let science_bin = find_science_bin()
        .ok_or_else(|| "找不到 claude-science 二进制。请通过 npm install -g @anthropic-ai/claude-science 安装。".to_string())?;

    let sbx_home = sandbox_home();
    fs::create_dir_all(&sbx_home)
        .map_err(|e| format!("创建沙箱 HOME 失败：{e}"))?;

    if let Some(t) = trace {
        t.stage(OperationStage::SandboxStart, "starting_science");
    }

    let proxy_url = format!("http://127.0.0.1:{proxy_port}/{secret}");

    let log_file = open_log("sandbox.log")
        .map_err(|e| format!("无法打开沙箱日志：{e}"))?;

    let mut cmd = Command::new(&science_bin);
    cmd.env("HOME", &sbx_home)
        .env("ANTHROPIC_BASE_URL", &proxy_url)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|e| format!("{e}"))?)
        .stderr(log_file);

    let mut child = cmd.spawn().map_err(|e| format!("启动 Claude Science 失败：{e}"))?;

    // Wait for health
    if let Some(t) = trace {
        t.stage(OperationStage::SandboxHealth, "waiting");
    }

    let mut ok = false;
    for _ in 0..((LOCAL_HEALTH_TIMEOUT_MS + 8000) / POLL_INTERVAL_MS) {
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        if sandbox_health(port) {
            ok = true;
            break;
        }
    }

    if !ok {
        let tail = redact(&tail_file(&log_path("sandbox.log"), 600), secret);
        let _ = child.kill();
        return Err(format!(
            "沙箱起后探活超时（端口 {port}）。\n{tail}"
        ));
    }

    Ok(child)
}

pub fn sandbox_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Stop the sandbox science process.
pub fn stop_science_sandbox(sandbox: &mut Option<Child>) {
    kill_child(sandbox);
}

/// Generate proxy environment variables for shell injection.
pub fn proxy_env_vars(proxy_port: u16, secret: &str) -> Vec<(String, String)> {
    let url = format!("http://127.0.0.1:{proxy_port}/{secret}");
    vec![
        ("ANTHROPIC_BASE_URL".to_string(), url),
        ("HOME".to_string(), sandbox_home().to_string_lossy().to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_url_returns_localhost_url() {
        assert_eq!(sandbox_url(8990), "http://127.0.0.1:8990");
    }

    #[test]
    fn proxy_env_vars_contains_anthropic_base_url() {
        let vars = proxy_env_vars(18991, "test-secret");
        assert_eq!(vars[0].0, "ANTHROPIC_BASE_URL");
        assert!(vars[0].1.contains("18991"));
        assert!(vars[0].1.contains("test-secret"));
    }

    #[test]
    fn sandbox_home_is_consistent() {
        let h = sandbox_home();
        assert!(h.to_string_lossy().contains(".csswitch"));
    }
}
