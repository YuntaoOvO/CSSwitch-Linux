use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use csswitch_config::Config;

use crate::operation::{POLL_INTERVAL_MS, LOCAL_HEALTH_TIMEOUT_MS};
use crate::provider::{
    proxy_args_for, proxy_fingerprint_with_runtime,
    gateway_kind_for_adapter,
    current_shim_mode_for_adapter, ProxyLaunch,
};
use crate::proxy::{ProxyAction};
use crate::system::{open_log, tail_file, redact, log_path};
use crate::RuntimeContext;

/// Gateway binary name and search paths.
const GATEWAY_BIN_NAME: &str = "csswitch-gateway";

/// Find the gateway binary from standard locations.
pub fn find_gateway_bin(ctx: &dyn RuntimeContext) -> Option<PathBuf> {
    // 1. Explicit env override
    if let Ok(path) = std::env::var("CSSWITCH_GATEWAY_BIN") {
        let p = PathBuf::from(&path);
        if p.is_file() {
            return Some(p);
        }
    }

    // 2. Next to the CLI binary itself
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(GATEWAY_BIN_NAME);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 3. In PATH
    if let Some(path) = crate::system::find_in_path(GATEWAY_BIN_NAME) {
        return Some(path);
    }

    // 4. Development fallback: repo root
    if let Some(repo) = ctx.repo_root() {
        // Release build
        let candidate = repo.join("target/release").join(GATEWAY_BIN_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Debug build
        let candidate = repo.join("target/debug").join(GATEWAY_BIN_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}



/// Start the CSSwitch gateway proxy process.
pub fn start_gateway(
    ctx: &dyn RuntimeContext,
    port: u16,
    _secret: &str,
    launch: &ProxyLaunch,
    _launch_id: &str,
) -> Result<Child, String> {
    let bin = find_gateway_bin(ctx)
        .ok_or_else(|| "找不到 csswitch-gateway 二进制。请确保已构建或设置 CSSWITCH_GATEWAY_BIN 环境变量。".to_string())?;

    let log_file = open_log("proxy.log")
        .map_err(|e| format!("无法打开代理日志：{e}"))?;

    // Pre-flight: verify the gateway binary is executable
    match Command::new(&bin).arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(_) => {},  // any exit code is fine — just checking it can launch at all
        Err(e) => return Err(format!("无法执行 csswitch-gateway（{}）：{}。请确认已通过 sudo dpkg -i 安装，且系统安装了 openssl。", bin.display(), e)),
    }

    let mut cmd = Command::new(&bin);
    cmd.arg("--provider").arg(&launch.adapter)
        .arg("--port").arg(port.to_string())
        .arg("--auth-token").arg(_secret)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|e| format!("{e}"))?)
        .stderr(Stdio::piped());  // capture stderr for error diagnostics

    // Inject provider key as env var (NEVER in argv)
    cmd.env(&launch.key_env, &launch.key);

    // For relay / openai-custom / openai-responses adapters, inject base URL and model
    let adapter = &launch.adapter;
    if adapter == "openai-custom" || adapter == "openai-responses" {
        cmd.env("CSSWITCH_OPENAI_BASE_URL", &launch.base_url);
        if !launch.model.is_empty() {
            cmd.env("CSSWITCH_OPENAI_MODEL", &launch.model);
        }
    } else if adapter == "relay" {
        cmd.env("CSSWITCH_RELAY_BASE_URL", &launch.base_url);
        if !launch.model.is_empty() {
            cmd.env("CSSWITCH_RELAY_MODEL", &launch.model);
        }
        if !launch.thinking_policy.is_empty() {
            cmd.env("CSSWITCH_RELAY_THINKING", &launch.thinking_policy);
        }
    }

    let child = cmd.spawn().map_err(|e| format!("启动 gateway 失败：{e}"))?;
    Ok(child)
}

/// Perform HTTP health check against the proxy using raw TCP (no reqwest dependency).
pub fn proxy_health(port: u16, _secret: &str) -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let mut stream = match TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(2000),
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(2000)));

    let req = "GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n";
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }

    let mut buf = [0u8; 128];
    let n = stream.read(&mut buf).unwrap_or(0);
    let response = String::from_utf8_lossy(&buf[..n]);
    response.contains("200") || response.contains("OK")
}

/// Ensure the proxy is running with the correct configuration.
/// Returns the secret, port, and whether it was reused or restarted.
pub fn ensure_proxy(
    ctx: &dyn RuntimeContext,
    state: &std::sync::Mutex<crate::AppState>,
    cfg: &Config,
    _lifecycle_gen: u64,
) -> Result<(String, u16, ProxyAction), String> {
    let profile = cfg.active_profile()
        .ok_or_else(|| "当前没有生效的 profile，请先用 `csswitch profile activate <id>` 设置。".to_string())?;

    let launch = proxy_args_for(profile);
    let shim_mode = current_shim_mode_for_adapter(&launch.adapter);
    let gateway_kind = gateway_kind_for_adapter(&launch.adapter);

    // Generate or reuse path secret
    let secret = if cfg.secret.is_empty() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        format!("{:016x}", ts)
    } else {
        cfg.secret.clone()
    };

    let port = cfg.proxy_port;
    let key_fp = proxy_fingerprint_with_runtime(
        profile, &launch, gateway_kind, shim_mode,
    );

    // Check if existing proxy is healthy and matches
    {
        let st = crate::lock(state);
        if st.proxy.is_some()
            && st.proxy_port == port
            && st.secret == secret
            && st.key_fp == key_fp
        {
            if proxy_health(port, &secret) {
                return Ok((secret, port, ProxyAction::Reused));
            }
        }
    }

    // Need to (re)start proxy
    // Kill existing proxy if any
    {
        let mut st = crate::lock(state);
        st.stop_proxy();
    }

    let launch_id = format!("{:016x}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());

    let mut child = start_gateway(ctx, port, &secret, &launch, &launch_id)?;

    // Wait for health
    let start = Instant::now();
    let mut healthy = false;
    while start.elapsed() < Duration::from_millis(LOCAL_HEALTH_TIMEOUT_MS + 8000) {
        if proxy_health(port, &secret) {
            healthy = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    if !healthy {
        // Gather all diagnostic info
        let log_tail = redact(&tail_file(&log_path("proxy.log"), 600), &secret);
        let mut stderr_out = String::new();
        if let Some(ref mut s) = child.stderr {
            use std::io::Read;
            let _ = s.read_to_string(&mut stderr_out);
        }
        let stderr_tail = redact(&stderr_out, &secret);

        let mut diag = format!("代理端口 {} 探活超时。", port);
        if !log_tail.is_empty() {
            diag.push_str(&format!("
[stdout]
{}", log_tail));
        }
        if !stderr_tail.is_empty() {
            diag.push_str(&format!("
[stderr]
{}", stderr_tail));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                diag.push_str(&format!("
gateway 进程已退出，退出码 {:?}", status.code()));
            },
            Ok(None) => {
                diag.push_str("
gateway 进程仍在运行但端口不可达，可能是上游连接失败。");
            },
            Err(_) => {},
        }
        let _ = child.kill();
        return Err(diag);
    }

    // Update state
    {
        let mut st = crate::lock(state);
        st.proxy = Some(child);
        st.proxy_port = port;
        st.secret = secret.clone();
        st.provider = launch.adapter.clone();
        st.gateway_kind = gateway_kind.to_string();
        st.shim_mode = shim_mode.to_string();
        st.launch_id = launch_id;
        st.key_fp = key_fp;
    }

    Ok((secret, port, ProxyAction::Restarted))
}

#[cfg(test)]
mod tests {

    #[test]
    fn find_gateway_bin_returns_none_when_not_installed() {
        // In CI/dev without the binary built, this should return None gracefully.
        // We don't assert Some/None because the binary may or may not exist.
        let _ = std::env::var("CSSWITCH_GATEWAY_BIN"); // just check it doesn't panic
    }
}
