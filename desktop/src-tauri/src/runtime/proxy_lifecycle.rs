use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use tauri::{Manager, Runtime};

use crate::runtime::operation::{self, OperationStage, OperationTrace, POLL_INTERVAL_MS};
use crate::runtime::provider::{
    assert_format_supported, current_shim_mode_for_adapter, gateway_kind_for_adapter,
    is_native_adapter, is_openai_adapter, proxy_args_for, proxy_fingerprint, ProxyLaunch,
};
use crate::runtime::proxy::{ere_escape, health_timeout_reason, should_write_back, ProxyAction};
use crate::runtime::system::{asset_root, log_path, open_log, redact, repo_root, tail_file};
use crate::{config, lifecycle, lock, proc, SharedAppState};

fn formal_proxy_env(launch: &ProxyLaunch) -> Vec<(&'static str, String)> {
    let native = is_native_adapter(&launch.adapter);
    let mut env = vec![(launch.key_env, launch.key.clone())];
    if !native {
        if is_openai_adapter(&launch.adapter) {
            env.push(("CSSWITCH_OPENAI_BASE_URL", launch.base_url.clone()));
            if !launch.model.is_empty() {
                env.push(("CSSWITCH_OPENAI_MODEL", launch.model.clone()));
            }
        } else {
            env.push(("CSSWITCH_RELAY_BASE_URL", launch.base_url.clone()));
            if !launch.model.is_empty() {
                env.push(("CSSWITCH_RELAY_MODEL", launch.model.clone()));
            }
            if !launch.thinking_policy.is_empty() {
                env.push((
                    "CSSWITCH_RELAY_THINKING",
                    launch.thinking_policy.to_string(),
                ));
            }
        }
    }
    env
}

fn find_gateway_in(dir: &Path) -> Option<PathBuf> {
    let exact = dir.join(if cfg!(windows) {
        "csswitch-gateway.exe"
    } else {
        "csswitch-gateway"
    });
    if exact.is_file() {
        return Some(exact);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let matches = if cfg!(windows) {
            name.starts_with("csswitch-gateway-") && name.ends_with(".exe")
        } else {
            name.starts_with("csswitch-gateway-")
        };
        if matches && path.is_file() {
            return Some(path);
        }
    }
    None
}

fn gateway_bin_path<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("CSSWITCH_GATEWAY_BIN") {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent().and_then(find_gateway_in) {
            return Some(dir);
        }
    }
    if let Ok(res) = app.path().resource_dir() {
        if let Some(path) = find_gateway_in(&res) {
            return Some(path);
        }
        if let Some(path) = find_gateway_in(&res.join("binaries")) {
            return Some(path);
        }
    }
    if let Some(root) = repo_root() {
        for dir in [
            root.join("desktop/gateway/target/release"),
            root.join("desktop/gateway/target/debug"),
            root.join("desktop/src-tauri/binaries"),
        ] {
            if let Some(path) = find_gateway_in(&dir) {
                return Some(path);
            }
        }
    }
    None
}

/// Ensure the active profile's proxy is running and healthy.
pub(crate) fn ensure_proxy<R: Runtime>(
    app: &tauri::AppHandle<R>,
    state: &SharedAppState,
    lifecycle: &lifecycle::Lifecycle,
    trace: Option<&OperationTrace>,
) -> Result<(u16, String, ProxyAction), String> {
    let cfg = config::load_from(&config::default_dir()).map_err(|e| e.to_string())?;
    let profile = cfg
        .active_profile()
        .cloned()
        .ok_or("未配置生效 profile，请先在面板选择或新建一条配置。")?;
    start_proxy_for(app, state, lifecycle, &profile, trace)
}

/// Start or reuse a proxy for a specific profile, without reading the active profile.
///
/// This function does not take the command serializer lock; callers own that boundary.
pub(crate) fn start_proxy_for<R: Runtime>(
    app: &tauri::AppHandle<R>,
    state: &SharedAppState,
    lifecycle: &lifecycle::Lifecycle,
    profile: &config::Profile,
    trace: Option<&OperationTrace>,
) -> Result<(u16, String, ProxyAction), String> {
    assert_format_supported(profile)?;
    let launch = proxy_args_for(profile);
    if launch.key.is_empty() {
        return Err(format!(
            "「{}」还没填 API key，请先在面板填写并保存。",
            profile.name
        ));
    }
    let native = is_native_adapter(&launch.adapter);
    if !native && launch.base_url.is_empty() {
        return Err(
            "该配置需要填 base_url（如 https://your-relay/claude），请先在面板填写并保存。".into(),
        );
    }

    let gateway_kind = gateway_kind_for_adapter(&launch.adapter);
    let shim_mode = current_shim_mode_for_adapter(&launch.adapter);
    let key_fp = proxy_fingerprint(profile, &launch);
    let dir = config::default_dir();
    let cfg = config::load_from(&dir).map_err(|e| e.to_string())?;
    let port = cfg.proxy_port;

    let secret = if !cfg.secret.is_empty() {
        cfg.secret.clone()
    } else {
        let s = proc::gen_secret().map_err(|e| format!("无法生成安全 secret：{e}"))?;
        let s2 = s.clone();
        config::update(&dir, move |c| c.secret = s2).map_err(|e| e.to_string())?;
        s
    };

    let gen = lifecycle.current_generation();

    let child = {
        let mut st = lock(state);
        if st.proxy.is_some()
            && st.proxy_port == port
            && st.provider == launch.adapter
            && st.gateway_kind == gateway_kind
            && st.shim_mode == shim_mode
            && st.key_fp == key_fp
            && proc::http_health(
                port,
                Some(&st.secret),
                operation::PROXY_REUSE_HEALTH_TIMEOUT_MS,
            )
        {
            if let Some(t) = trace {
                t.stage(
                    OperationStage::ProxyHealth,
                    format!(
                        "reused port={port} adapter={} gateway={gateway_kind}",
                        launch.adapter
                    ),
                );
            }
            return Ok((port, st.secret.clone(), ProxyAction::Reused));
        }

        st.stop_proxy();
        st.secret = secret.clone();

        let logf = open_log("proxy.log").map_err(|e| format!("建日志失败：{e}"))?;
        let logf2 = logf.try_clone().map_err(|e| e.to_string())?;
        if let Some(t) = trace {
            t.stage(
                OperationStage::ProxySpawn,
                format!(
                    "port={port} adapter={} gateway={gateway_kind}",
                    launch.adapter
                ),
            );
        }
        let mut cmd = if gateway_kind == "rust" {
            let bin = gateway_bin_path(app)
                .ok_or("找不到 csswitch-gateway 二进制；请先构建 desktop/gateway，或设置 CSSWITCH_GATEWAY_BIN。")?;
            let mut cmd = Command::new(bin);
            cmd.arg("--provider")
                .arg("deepseek")
                .arg("--port")
                .arg(port.to_string())
                .arg("--auth-token")
                .arg(&secret)
                .env(launch.key_env, &launch.key);
            cmd
        } else {
            let root = asset_root(app)
                .ok_or("找不到代理脚本 proxy/csswitch_proxy.py（打包资源或仓库根均未命中）。开发态可设 CSSWITCH_REPO。")?;
            let py = proc::find_exe("python3")
                .ok_or("缺少依赖 python3（起翻译代理需要）。已查 PATH、常见目录与登录 shell 仍未找到；macOS 一般自带 /usr/bin/python3（装 Xcode 命令行工具：xcode-select --install）。")?;
            let script = root.join("proxy/csswitch_proxy.py");
            let pat = format!("{}.*--port {port}", ere_escape(&script.to_string_lossy()));
            let _ = Command::new("pkill").arg("-f").arg(&pat).status();
            let mut cmd = Command::new(&py);
            cmd.arg(&script)
                .arg("--provider")
                .arg(&launch.adapter)
                .arg("--port")
                .arg(port.to_string())
                .arg("--auth-token")
                .arg(&secret);
            for (k, v) in formal_proxy_env(&launch) {
                cmd.env(k, v);
            }
            cmd
        };
        if gateway_kind == "rust" {
            cmd.env("CSSWITCH_AUTH_TOKEN", &secret);
        }
        cmd.stdout(Stdio::from(logf))
            .stderr(Stdio::from(logf2))
            .spawn()
            .map_err(|e| format!("启动代理失败：{e}"))?
    };

    let mut ok = false;
    for _ in 0..(operation::PROXY_HEALTH_BUDGET_MS / POLL_INTERVAL_MS) {
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        if proc::http_health(port, Some(&secret), operation::LOCAL_HEALTH_TIMEOUT_MS) {
            ok = true;
            break;
        }
    }
    if let Some(t) = trace {
        t.stage(
            OperationStage::ProxyHealth,
            if ok { "ready" } else { "not_ready" },
        );
    }
    if !ok {
        let mut c = child;
        let _ = c.kill();
        let _ = c.wait();
        let tail = redact(&tail_file(&log_path("proxy.log"), 500), &secret);
        return Err(format!("{}\n{tail}", health_timeout_reason(port, &tail)));
    }

    {
        let mut st = lock(state);
        if !should_write_back(gen, lifecycle.current_generation(), &st.secret, &secret) {
            let mut c = child;
            let _ = c.kill();
            let _ = c.wait();
            return Err("代理启动期间配置已变更（被更晚的操作取代），本次启动未生效。".into());
        }
        st.proxy = Some(child);
        st.proxy_port = port;
        st.secret = secret.clone();
        st.provider = launch.adapter.clone();
        st.gateway_kind = gateway_kind.to_string();
        st.shim_mode = shim_mode.to_string();
        st.key_fp = key_fp;
    }
    Ok((port, secret, ProxyAction::Restarted))
}

#[cfg(test)]
mod tests {
    use super::{find_gateway_in, formal_proxy_env};
    use crate::runtime::provider::ProxyLaunch;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn launch(adapter: &str, model: &str) -> ProxyLaunch {
        launch_with_thinking(adapter, model, "")
    }

    fn launch_with_thinking(
        adapter: &str,
        model: &str,
        thinking_policy: &'static str,
    ) -> ProxyLaunch {
        ProxyLaunch {
            adapter: adapter.to_string(),
            base_url: "https://upstream.example/api".to_string(),
            model: model.to_string(),
            key: "test-key".to_string(),
            key_env: if matches!(adapter, "openai-custom" | "openai-responses") {
                "CSSWITCH_OPENAI_KEY"
            } else {
                "CSSWITCH_RELAY_KEY"
            },
            thinking_policy,
        }
    }

    #[test]
    fn formal_proxy_env_pins_relay_model_only_on_formal_launch() {
        let env = formal_proxy_env(&launch("relay", "glm-5.2"));
        assert!(env.contains(&(
            "CSSWITCH_RELAY_BASE_URL",
            "https://upstream.example/api".to_string()
        )));
        assert!(env.contains(&("CSSWITCH_RELAY_MODEL", "glm-5.2".to_string())));
    }

    #[test]
    fn formal_proxy_env_pins_openai_model_only_on_formal_launch() {
        let env = formal_proxy_env(&launch("openai-custom", "gpt-5.2"));
        assert!(env.contains(&(
            "CSSWITCH_OPENAI_BASE_URL",
            "https://upstream.example/api".to_string()
        )));
        assert!(env.contains(&("CSSWITCH_OPENAI_MODEL", "gpt-5.2".to_string())));
    }

    #[test]
    fn formal_proxy_env_native_adapter_only_sets_native_key() {
        let mut native = launch("deepseek", "");
        native.key_env = "DEEPSEEK_API_KEY";
        let env = formal_proxy_env(&native);
        assert_eq!(env, vec![("DEEPSEEK_API_KEY", "test-key".to_string())]);
    }

    #[test]
    fn formal_proxy_env_empty_model_does_not_pin_model() {
        let env = formal_proxy_env(&launch("relay", ""));
        assert!(env.iter().any(|(k, _)| *k == "CSSWITCH_RELAY_BASE_URL"));
        assert!(!env.iter().any(|(k, _)| *k == "CSSWITCH_RELAY_MODEL"));
        assert!(!env.iter().any(|(k, _)| *k == "CSSWITCH_OPENAI_MODEL"));
    }

    #[test]
    fn formal_proxy_env_preserves_relay_thinking_policy() {
        let env = formal_proxy_env(&launch_with_thinking("relay", "glm-5.2", "enabled"));
        assert!(env.contains(&("CSSWITCH_RELAY_THINKING", "enabled".to_string())));
    }

    #[test]
    fn find_gateway_in_accepts_plain_or_tauri_suffixed_binary() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "csswitch-gateway-find-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let name = if cfg!(windows) {
            "csswitch-gateway-aarch64-pc-windows-msvc.exe"
        } else {
            "csswitch-gateway-aarch64-apple-darwin"
        };
        let path = dir.join(name);
        fs::write(&path, b"bin").unwrap();
        assert_eq!(find_gateway_in(&dir), Some(path.clone()));
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(dir);
    }
}
