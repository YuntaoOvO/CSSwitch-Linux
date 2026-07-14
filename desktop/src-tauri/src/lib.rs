//! CSSwitch 桌面 app 后端（CLI 图形包装层）。
//!
//! **架构原则：Desktop = CLI 的 GUI 皮。** Desktop 不自己 spawn 代理/网关进程，
//! 全部委托给 `csswitch daemon start/stop/status`。端口冲突天然解决（daemon 是全局
//! 单例），关闭窗口 = 最小化到托盘（daemon 独立运行），其他 shell 正常 `eval "$(csswitch env)"`。
//!
//! Desktop 自身保留的逻辑只有：
//! - Profile CRUD 面板（读写 config.json）
//! - 上游 scratch 校验（切换前预检 key）
//! - 虚拟 OAuth 登录伪造（沙箱隔离登录态）
//! - Science 沙箱启动/停止（隔离 HOME + 端口）
//! - 系统托盘状态显示 + 右键菜单
//!
//! 铁律：key 只在内存与 0600 的 config.json；回显前端只给掩码；沙箱端口/目录护栏
//! 由被调脚本负责（对 8765 与真实目录失败关闭）。

mod config;
mod config_legacy;
mod lifecycle;
mod oauth_forge;
mod proc;
mod scratch;
mod templates;
mod tray;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use tauri::{Manager, State};

/// Science 二进制路径：优先 `SCIENCE_BIN` 环境变量，否则按平台默认。
fn science_bin() -> String {
    if let Ok(s) = std::env::var("SCIENCE_BIN") {
        if !s.is_empty() {
            return s;
        }
    }
    if cfg!(target_os = "macos") {
        "/Applications/Claude Science.app/Contents/Resources/bin/claude-science".to_string()
    } else {
        if let Ok(home) = std::env::var("HOME") {
            let default = format!("{home}/.local/bin/claude-science");
            if Path::new(&default).is_file() {
                return default;
            }
        }
        "/usr/local/bin/claude-science".to_string()
    }
}

/// 用于跑脚本的 shell：优先 $SHELL，回退 bash。
fn system_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

// ---------- AppState：只追踪沙箱，代理全委托给 CLI daemon ----------

#[derive(Default)]
struct AppState {
    sandbox: Option<Child>,
    sandbox_port: u16,
    sandbox_url: Option<String>,
}

// ---------- adapter / profile 运行元信息（scratch 校验复用）----------
fn key_env_for_adapter(adapter: &str) -> &'static str {
    match adapter {
        "deepseek" => "DEEPSEEK_API_KEY",
        "qwen" => "DASHSCOPE_API_KEY",
        _ => "CSSWITCH_RELAY_KEY",
    }
}

struct ProxyLaunch {
    adapter: String,
    base_url: String,
    model: String,
    key: String,
    key_env: &'static str,
    thinking_policy: &'static str,
}

fn proxy_args_for(p: &config::Profile) -> ProxyLaunch {
    let adapter = templates::adapter_for(&p.template_id).to_string();
    let key_env = key_env_for_adapter(&adapter);
    ProxyLaunch {
        adapter,
        base_url: p.base_url.clone(),
        model: p.model.clone(),
        key: p.api_key.clone(),
        key_env,
        thinking_policy: templates::thinking_policy_for(&p.template_id),
    }
}

fn assert_format_supported(p: &config::Profile) -> Result<(), String> {
    match p.api_format.as_str() {
        "anthropic" | "openai_chat" => Ok(()),
        other => Err(format!(
            "api_format `{other}` 暂不支持（待 Rust 代理），请选 anthropic 或 openai_chat。"
        )),
    }
}

fn is_native_adapter(adapter: &str) -> bool {
    adapter == "deepseek" || adapter == "qwen"
}

fn upstream_host(adapter: &str, base_url: &str) -> String {
    match adapter {
        "deepseek" => "api.deepseek.com".to_string(),
        "qwen" => "dashscope.aliyuncs.com".to_string(),
        _ => parse_host(base_url).unwrap_or_default(),
    }
}

fn parse_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or("")
        .to_string();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

fn is_main_list_model(id: &str) -> bool {
    for fam in ["claude-opus-", "claude-sonnet-", "claude-haiku-"] {
        if let Some(rest) = id.strip_prefix(fam) {
            return rest
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
        }
    }
    false
}

// ---------- 路径与日志 ----------
fn repo_root() -> Option<PathBuf> {
    let marker = Path::new("proxy/csswitch_proxy.py");
    if let Some(r) = std::env::var_os("CSSWITCH_REPO") {
        if let Ok(p) = std::fs::canonicalize(PathBuf::from(r)) {
            if p.join(marker).is_file() {
                return Some(p);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir: Option<&Path> = exe.parent();
        while let Some(d) = dir {
            if d.join(marker).is_file() {
                return Some(d.to_path_buf());
            }
            dir = d.parent();
        }
    }
    None
}

fn asset_root(app: &tauri::AppHandle) -> Option<PathBuf> {
    let marker = Path::new("proxy/csswitch_proxy.py");
    if let Ok(res) = app.path().resource_dir() {
        if res.join(marker).is_file() {
            return Some(res);
        }
    }
    repo_root()
}

fn sandbox_home() -> PathBuf {
    config::default_dir().join("sandbox").join("home")
}

fn log_path(name: &str) -> PathBuf {
    config::default_dir().join("logs").join(name)
}

const fn libc_o_nofollow() -> i32 {
    if cfg!(target_os = "linux") {
        0x2_0000
    } else {
        0x0100
    }
}

fn open_log(name: &str) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let p = log_path(name);
    if let Some(parent) = p.parent() {
        config::assert_not_symlink(parent)?;
        std::fs::create_dir_all(parent)?;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    config::assert_not_symlink(&p)?;
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc_o_nofollow())
        .open(&p)?;
    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    Ok(f)
}

fn redact(s: &str, secret: &str) -> String {
    if secret.is_empty() {
        s.to_string()
    } else {
        s.replace(secret, "****")
    }
}

fn tail_file(path: &Path, max: usize) -> String {
    match std::fs::read(path) {
        Ok(b) => {
            let start = b.len().saturating_sub(max);
            String::from_utf8_lossy(&b[start..]).trim().to_string()
        }
        Err(_) => String::new(),
    }
}

fn kill_child(slot: &mut Option<Child>) {
    if let Some(mut c) = slot.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn lock(m: &Mutex<AppState>) -> std::sync::MutexGuard<'_, AppState> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn open_in_browser(url: &str) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let st = Command::new(opener)
        .arg(url)
        .status()
        .map_err(|e| format!("打开浏览器失败：{e}"))?;
    if !st.success() {
        return Err(format!("{opener} 非零退出（{:?}）", st.code()));
    }
    Ok(())
}

// ---------- CLI daemon 委托 ----------

/// 确保 daemon 在跑：调用 `csswitch daemon start`（幂等），然后从 config.json 读端口+secret。
/// 返回 (port, secret, "已在运行" | "已启动")。
fn ensure_daemon_via_cli() -> Result<(u16, String, String), String> {
    let output = Command::new("csswitch")
        .args(["daemon", "start"])
        .output()
        .map_err(|e| format!("执行 csswitch daemon start 失败：{e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        return Err(format!("csswitch daemon start 失败：{}", stderr.trim()));
    }

    // daemon start 之后 secret 已持久化到 config.json，读出来
    let dir = config::default_dir();
    let cfg = config::load_from(&dir).map_err(|e| format!("读取配置失败：{e}"))?;

    if cfg.secret.is_empty() {
        return Err("daemon 启动后 secret 为空，配置异常。".to_string());
    }

    // 从 stdout 判断是复用还是重启
    let action = if stdout.contains("已在运行") {
        "已在运行"
    } else {
        "已启动"
    };

    Ok((cfg.proxy_port, cfg.secret, action.to_string()))
}

// ---------- 沙箱管理 ----------

fn stop_sandbox_inner(app: &tauri::AppHandle, st: &mut AppState) -> Result<(), String> {
    let mut err = None;
    match asset_root(app) {
        Some(root) => {
            let stop = root.join("scripts/stop-science-sandbox.sh");
            if stop.is_file() {
                let shell = system_shell();
                match Command::new(&shell)
                    .arg(&stop)
                    .env("SANDBOX_HOME", sandbox_home())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                {
                    Ok(s) if s.success() => {}
                    Ok(s) => err = Some(format!("停止沙箱脚本非零退出（{:?}）。", s.code())),
                    Err(e) => err = Some(format!("调用停止沙箱脚本失败：{e}")),
                }
            } else {
                err = Some(format!(
                    "找不到停止脚本 {}，无法确认沙箱已停止（沙箱可能仍在运行）。",
                    stop.display()
                ));
            }
        }
        None => {
            err = Some(
                "定位不到资源根，取不到停止脚本，无法确认沙箱已停止（沙箱可能仍在运行）。"
                    .to_string(),
            );
        }
    }
    kill_child(&mut st.sandbox);
    st.sandbox_url = None;
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn sandbox_url(port: u16) -> String {
    let home = sandbox_home();
    let data_dir = home.join(".claude-science");
    let bin = science_bin();
    if Path::new(&bin).is_file() {
        if let Ok(out) = Command::new(&bin)
            .arg("url")
            .arg("--data-dir")
            .arg(&data_dir)
            .env("HOME", &home)
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(url) = first_http_url(&s) {
                return url;
            }
        }
    }
    format!("http://127.0.0.1:{port}")
}

fn first_http_url(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let t = line.trim();
        if t.starts_with("http://") || t.starts_with("https://") {
            let url = t.split_whitespace().next().unwrap_or(t);
            return Some(url.to_string());
        }
    }
    None
}

fn sandbox_running_ours(port: u16) -> bool {
    let home = sandbox_home();
    let data_dir = home.join(".claude-science");
    let bin = science_bin();
    if Path::new(&bin).is_file() {
        match Command::new(&bin)
            .arg("status")
            .arg("--data-dir")
            .arg(&data_dir)
            .env("HOME", &home)
            .output()
        {
            Ok(out) => {
                let s = String::from_utf8_lossy(&out.stdout);
                let running = s.contains("\"running\":true") || s.contains("\"running\": true");
                return running && proc::http_health(port, None, 400);
            }
            Err(_) => return proc::http_health(port, None, 400),
        }
    }
    proc::http_health(port, None, 400)
}

// ---------- 返回体组装 ----------
fn build_get_config(dir: &Path) -> Result<serde_json::Value, String> {
    let cfg = config::load_from(dir).map_err(|e| e.to_string())?;
    let notice = cfg.pending_notice.clone();
    if notice.is_some() {
        config::update(dir, |c| c.pending_notice = None).map_err(|e| e.to_string())?;
    }
    let profiles: Vec<serde_json::Value> = cfg
        .profiles
        .iter()
        .map(|p| {
            json!({
                "id": p.id, "name": p.name, "template_id": p.template_id, "category": p.category,
                "api_format": p.api_format, "base_url": p.base_url, "model": p.model,
                "key": config::mask(&p.api_key), "icon": p.icon, "icon_color": p.icon_color,
                "website_url": p.website_url, "sort_index": p.sort_index, "notes": p.notes,
            })
        })
        .collect();
    Ok(json!({
        "schema_version": cfg.schema_version, "active_id": cfg.active_id, "profiles": profiles,
        "templates": build_list_templates(), "proxy_port": cfg.proxy_port,
        "sandbox_port": cfg.sandbox_port, "mode": cfg.mode, "pending_notice": notice,
    }))
}

fn build_list_templates() -> Vec<serde_json::Value> {
    templates::all()
        .iter()
        .map(|t| {
            json!({
                "id": t.id, "name": t.name, "category": t.category, "api_format": t.api_format,
                "adapter": t.adapter, "base_url": t.base_url, "base_url_editable": t.base_url_editable,
                "requires_model_override": t.requires_model_override,
                "builtin_models": t.builtin_models, "icon": t.icon, "icon_color": t.icon_color,
                "website_url": t.website_url,
            })
        })
        .collect()
}

// ---------- profile CRUD 纯实现 ----------
fn create_profile_inner(
    dir: &Path,
    template_id: &str,
    name: &str,
    key: Option<&str>,
    base_url_override: Option<&str>,
    model: Option<&str>,
) -> Result<String, String> {
    let tpl = templates::by_id(template_id).ok_or_else(|| format!("未知模板：{template_id}"))?;
    let id = config::new_id();
    let base_url = base_url_override
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| tpl.base_url.to_string());
    let p = config::Profile {
        id: id.clone(),
        name: name.to_string(),
        template_id: template_id.to_string(),
        category: tpl.category.to_string(),
        api_format: tpl.api_format.to_string(),
        base_url,
        api_key: key.unwrap_or("").to_string(),
        model: model.unwrap_or("").to_string(),
        website_url: Some(tpl.website_url.to_string()),
        icon: Some(tpl.icon.to_string()),
        icon_color: Some(tpl.icon_color.to_string()),
        sort_index: Some(config::now_ms()),
        created_at: Some(config::now_ms()),
        notes: None,
    };
    assert_format_supported(&p)?;
    if relay_missing_model(tpl.adapter, &p.model) {
        return Err("中转 / 自定义端点必须选择或填写一个模型，未创建。".to_string());
    }
    config::update(dir, |c| c.profiles.push(p)).map_err(|e| e.to_string())?;
    Ok(id)
}

fn update_profile_metadata_inner(
    dir: &Path,
    id: &str,
    name: &str,
    notes: Option<&str>,
) -> Result<(), String> {
    if config::load_from(dir)
        .map_err(|e| e.to_string())?
        .profile_by_id(id)
        .is_none()
    {
        return Err(format!("找不到 profile：{id}"));
    }
    config::update(dir, |c| {
        if let Some(p) = c.profile_by_id_mut(id) {
            p.name = name.to_string();
            p.notes = notes.map(str::to_string);
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn clear_profile_key_inner(dir: &Path, id: &str) -> Result<(), String> {
    config::update(dir, |c| {
        if let Some(p) = c.profile_by_id_mut(id) {
            p.api_key.clear();
        }
    })
    .map_err(|e| e.to_string())?;
    config::drop_rolling_backup(dir);
    Ok(())
}

fn delete_profile_inner(dir: &Path, id: &str) -> Result<(), String> {
    config::update(dir, |c| {
        c.profiles.retain(|p| p.id != id);
        if c.active_id == id {
            c.active_id.clear();
        }
    })
    .map_err(|e| e.to_string())?;
    config::drop_rolling_backup(dir);
    Ok(())
}

fn update_profile_connection_inner(
    dir: &Path,
    id: &str,
    base_url: Option<&str>,
    api_format: Option<&str>,
    model: Option<&str>,
    key: Option<&str>,
) -> Result<(), String> {
    if let Some(fmt) = api_format {
        let probe = config::Profile {
            api_format: fmt.to_string(),
            ..Default::default()
        };
        assert_format_supported(&probe)?;
    }
    if config::load_from(dir)
        .map_err(|e| e.to_string())?
        .profile_by_id(id)
        .is_none()
    {
        return Err(format!("找不到 profile：{id}"));
    }
    config::write_rolling_backup(dir).ok();
    config::update(dir, |c| {
        if let Some(p) = c.profile_by_id_mut(id) {
            if let Some(u) = base_url {
                p.base_url = u.to_string();
            }
            if let Some(f) = api_format {
                p.api_format = f.to_string();
            }
            if let Some(m) = model {
                p.model = m.to_string();
            }
            if let Some(k) = key {
                if !k.is_empty() {
                    p.api_key = k.to_string();
                }
            }
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- 守卫（纯函数）----------
fn relay_missing_base_url(adapter: &str, base_url: &str) -> bool {
    !is_native_adapter(adapter) && base_url.trim().is_empty()
}

fn relay_missing_model(adapter: &str, model: &str) -> bool {
    !is_native_adapter(adapter) && model.trim().is_empty()
}

fn should_scratch_candidate(adapter: &str, key: &str, base_url: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    if !is_native_adapter(adapter) && base_url.is_empty() {
        return false;
    }
    true
}

fn skip_scratch_verify(native: bool, skip_verify: bool) -> bool {
    let _ = native;
    skip_verify
}

fn nonactive_probe_verdict(outcome: &scratch::ProbeOutcome) -> Result<bool, String> {
    match outcome {
        scratch::ProbeOutcome::Ok => Ok(true),
        scratch::ProbeOutcome::Auth(code) => {
            Err(format!("上游拒绝（{code}），key/权限有误，连接未保存。"))
        }
        scratch::ProbeOutcome::ModelError(code) => Err(format!(
            "上游拒绝该模型（{code}），连接未保存。请换一个模型或核对 base_url。"
        )),
        scratch::ProbeOutcome::Ambiguous(_)
        | scratch::ProbeOutcome::NoResponse
        | scratch::ProbeOutcome::Unsupported(_) => Ok(false),
    }
}

fn probe_kind_for(adapter: &str, model: &str) -> scratch::ProbeKind {
    if is_native_adapter(adapter) {
        return scratch::ProbeKind::Message;
    }
    probe_kind_for_model(model)
}

fn probe_kind_for_model(model: &str) -> scratch::ProbeKind {
    if model.trim().is_empty() {
        scratch::ProbeKind::Models
    } else {
        scratch::ProbeKind::Message
    }
}

fn settings_change_needs_teardown(
    old_proxy: u16, new_proxy: u16,
    old_sandbox: u16, new_sandbox: u16,
) -> bool {
    old_proxy != new_proxy || old_sandbox != new_sandbox
}

fn scratch_validate_candidate(
    app: &tauri::AppHandle,
    candidate: &config::Profile,
) -> Result<bool, String> {
    let launch = proxy_args_for(candidate);
    if !should_scratch_candidate(&launch.adapter, &launch.key, &launch.base_url) {
        return Ok(false);
    }
    let root = asset_root(app).ok_or("找不到代理脚本 proxy/csswitch_proxy.py。")?;
    let py = proc::find_exe("python3").ok_or("缺少依赖 python3（起临时代理需要）。")?;
    let script = root.join("proxy/csswitch_proxy.py");
    let res = scratch::scratch_probe(
        &py,
        &script,
        &scratch::ScratchTarget {
            provider: &launch.adapter,
            key_env: launch.key_env,
            base_url: &launch.base_url,
            key: &launch.key,
            model: Some(&launch.model),
            relay_thinking: launch.thinking_policy,
        },
        probe_kind_for(&launch.adapter, &launch.model),
    );
    nonactive_probe_verdict(&scratch::classify(res.status))
}

fn merge_and_sort_models(
    live: Vec<(String, Option<bool>)>,
    builtin: &[&str],
) -> Vec<serde_json::Value> {
    let mut seen = std::collections::BTreeSet::new();
    let mut merged: Vec<(String, Option<bool>)> = Vec::new();
    for (id, st) in live {
        if seen.insert(id.clone()) {
            merged.push((id, st));
        }
    }
    for b in builtin {
        if seen.insert(b.to_string()) {
            merged.push((b.to_string(), None));
        }
    }
    merged.sort_by_key(|(id, st)| {
        let cap = match st {
            Some(true) => 0u8,
            None => 1,
            Some(false) => 2,
        };
        let main = if is_main_list_model(id) { 0u8 } else { 1 };
        (cap, main)
    });
    merged
        .into_iter()
        .map(|(id, st)| json!({ "id": id, "supports_tools": st }))
        .collect()
}

fn resolve_probe_key(profile_id: Option<&str>, candidate: &str) -> Result<String, String> {
    let c = candidate.trim();
    if !c.is_empty() {
        return Ok(c.to_string());
    }
    let pid = profile_id.ok_or("请先填写 API Key / Token。")?;
    let cfg = config::load_from(&config::default_dir()).map_err(|e| e.to_string())?;
    cfg.profile_by_id(pid)
        .map(|p| p.api_key.clone())
        .filter(|k| !k.is_empty())
        .ok_or_else(|| "请先填写 API Key / Token。".to_string())
}

// ---------- 连接编辑（validate-before-persist）----------
#[derive(Default)]
struct ConnectionEdit {
    base_url: Option<String>,
    api_format: Option<String>,
    model: Option<String>,
    key: Option<String>,
}

impl ConnectionEdit {
    fn apply(&self, p: &mut config::Profile) {
        if let Some(u) = &self.base_url {
            p.base_url = u.clone();
        }
        if let Some(f) = &self.api_format {
            p.api_format = f.clone();
        }
        if let Some(m) = &self.model {
            p.model = m.clone();
        }
        if let Some(k) = &self.key {
            if !k.is_empty() {
                p.api_key = k.clone();
            }
        }
    }
}

// ========== Tauri commands ==========

#[tauri::command]
fn get_config() -> Result<serde_json::Value, String> {
    build_get_config(&config::default_dir())
}

#[tauri::command]
fn list_templates() -> Vec<serde_json::Value> {
    build_list_templates()
}

/// 切换运行模式。切「官方」→ 调 CLI 停 daemon + 停沙箱；daemon 由 CLI 管理。
#[tauri::command]
fn set_mode(
    app: tauri::AppHandle,
    state: State<'_, Mutex<AppState>>,
    lifecycle: State<'_, lifecycle::Lifecycle>,
    mode: String,
) -> Result<(), String> {
    if mode != "proxy" && mode != "official" {
        return Err(format!("未知模式：{mode}（只支持 proxy / official）。"));
    }
    lifecycle.with_serialized(|| {
        let dir = config::default_dir();
        if mode == "official" {
            lifecycle.bump_generation();
            // 经 CLI 停 daemon
            let _ = Command::new("csswitch")
                .args(["daemon", "stop"])
                .output();
            let mut st = lock(&state);
            stop_sandbox_inner(&app, &mut st).map_err(|e| {
                format!("停止沙箱失败，未切换到官方模式：{e}（真实实例 8765 未受影响）")
            })?;
        }
        config::update(&dir, {
            let mode = mode.clone();
            move |c| c.mode = mode
        })
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
fn open_official() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let app_path = "/Applications/Claude Science.app";
        let mut cmd = Command::new("open");
        if Path::new(app_path).is_dir() {
            cmd.arg(app_path);
        } else {
            cmd.arg("-a").arg("Claude Science");
        }
        cmd.env_remove("ANTHROPIC_BASE_URL")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN");
        match cmd.status() {
            Ok(s) if s.success() => Ok(()),
            Ok(_) => Err("未能打开 Claude Science。请确认已安装官方 Claude Science。".into()),
            Err(e) => Err(format!("打开官方 Claude Science 失败：{e}")),
        }
    } else {
        let bin = science_bin();
        let mut cmd = Command::new(&bin);
        cmd.env_remove("ANTHROPIC_BASE_URL")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN");
        match cmd.spawn() {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("启动 Claude Science 失败（{bin}）：{e}")),
        }
    }
}

#[derive(Deserialize)]
struct UiSettings {
    proxy_port: u16,
    sandbox_port: u16,
}

/// 端口设置。变更→经 CLI 停 daemon + 停沙箱；daemon 由 CLI 下次 start 重建。
#[tauri::command]
fn set_settings(
    app: tauri::AppHandle,
    state: State<'_, Mutex<AppState>>,
    lifecycle: State<'_, lifecycle::Lifecycle>,
    cfg: UiSettings,
) -> Result<(), String> {
    if cfg.proxy_port == 8765 || cfg.sandbox_port == 8765 {
        return Err("端口 8765 是真实 Science 实例保留端口，不能用。".into());
    }
    if cfg.proxy_port == 0 || cfg.sandbox_port == 0 {
        return Err("端口不能为 0。".into());
    }
    if cfg.proxy_port == cfg.sandbox_port {
        return Err("代理端口与沙箱端口不能相同。".into());
    }
    lifecycle.with_serialized(|| {
        let dir = config::default_dir();
        let old = config::load_from(&dir).map_err(|e| e.to_string())?;
        if settings_change_needs_teardown(old.proxy_port, cfg.proxy_port, old.sandbox_port, cfg.sandbox_port) {
            let mut st = lock(&state);
            stop_sandbox_inner(&app, &mut st).map_err(|e| {
                format!(
                    "端口未更改：无法停止指向旧端口的沙箱（{e}），为避免留下失效链路，端口保持不变。请手动停止沙箱或重启 app 后重试。"
                )
            })?;
            lifecycle.bump_generation();
            // 经 CLI 停 daemon（旧端口）
            let _ = Command::new("csswitch")
                .args(["daemon", "stop"])
                .output();
        }
        config::update(&dir, move |c| {
            c.proxy_port = cfg.proxy_port;
            c.sandbox_port = cfg.sandbox_port;
        })
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ---------- profile CRUD 命令 ----------
#[tauri::command]
fn create_profile(
    lifecycle: State<'_, lifecycle::Lifecycle>,
    template_id: String,
    name: String,
    key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
) -> Result<String, String> {
    lifecycle.with_serialized(|| {
        create_profile_inner(
            &config::default_dir(),
            &template_id,
            &name,
            key.as_deref(),
            base_url.as_deref(),
            model.as_deref(),
        )
    })
}

#[tauri::command]
fn update_profile_metadata(
    lifecycle: State<'_, lifecycle::Lifecycle>,
    id: String,
    name: String,
    notes: Option<String>,
) -> Result<(), String> {
    lifecycle.with_serialized(|| {
        update_profile_metadata_inner(&config::default_dir(), &id, &name, notes.as_deref())
    })
}

/// 清 key：若清的是 active → 经 CLI 停 daemon（旧 key 不再服务）。
#[tauri::command]
fn clear_profile_key(
    lifecycle: State<'_, lifecycle::Lifecycle>,
    id: String,
) -> Result<(), String> {
    lifecycle.with_serialized(|| {
        let dir = config::default_dir();
        let was_active = config::load_from(&dir)
            .map(|c| c.active_id == id)
            .unwrap_or(false);
        clear_profile_key_inner(&dir, &id)?;
        if was_active {
            lifecycle.bump_generation();
            let _ = Command::new("csswitch")
                .args(["daemon", "stop"])
                .output();
        }
        Ok(())
    })
}

#[tauri::command]
fn delete_profile(
    lifecycle: State<'_, lifecycle::Lifecycle>,
    id: String,
) -> Result<(), String> {
    lifecycle.with_serialized(|| {
        let dir = config::default_dir();
        let was_active = config::load_from(&dir)
            .map(|c| c.active_id == id)
            .unwrap_or(false);
        delete_profile_inner(&dir, &id)?;
        if was_active {
            lifecycle.bump_generation();
            let _ = Command::new("csswitch")
                .args(["daemon", "stop"])
                .output();
        }
        Ok(())
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn update_profile_connection(
    app: tauri::AppHandle,
    lifecycle: State<'_, lifecycle::Lifecycle>,
    id: String,
    base_url: Option<String>,
    api_format: Option<String>,
    model: Option<String>,
    key: Option<String>,
) -> Result<serde_json::Value, String> {
    lifecycle.with_serialized(|| {
        let dir = config::default_dir();
        let cfg = config::load_from(&dir).map_err(|e| e.to_string())?;
        let mut candidate = cfg
            .profile_by_id(&id)
            .cloned()
            .ok_or_else(|| format!("找不到 profile：{id}"))?;
        let edit = ConnectionEdit {
            base_url: base_url.clone(),
            api_format: api_format.clone(),
            model: model.clone(),
            key: key.clone(),
        };
        edit.apply(&mut candidate);
        if relay_missing_base_url(
            templates::adapter_for(&candidate.template_id),
            &candidate.base_url,
        ) {
            return Err("中转 / 自定义端点必须填写连接地址（base_url），连接未保存。".to_string());
        }
        if relay_missing_model(
            templates::adapter_for(&candidate.template_id),
            &candidate.model,
        ) {
            return Err("中转 / 自定义端点必须选择或填写一个模型，连接未保存。".to_string());
        }
        if cfg.active_id == id {
            // active：validate-before-persist → 经 CLI 切 daemon
            let v = set_active_profile_txn(&app, lifecycle.inner(), &id, false, Some(&edit))?;
            if v.get("committed").and_then(|b| b.as_bool()) == Some(false) {
                let hint = v
                    .get("hint")
                    .and_then(|h| h.as_str())
                    .unwrap_or("连接校验未通过，连接未保存。")
                    .to_string();
                return Err(hint);
            }
            Ok(json!({ "validated": true }))
        } else {
            let validated = scratch_validate_candidate(&app, &candidate)?;
            update_profile_connection_inner(
                &dir,
                &id,
                base_url.as_deref(),
                api_format.as_deref(),
                model.as_deref(),
                key.as_deref(),
            )?;
            Ok(json!({ "validated": validated }))
        }
    })
}

// ---------- 激活 / 切换 ----------

/// 切换事务：scratch 校验候选 → 更新 active_id + 连接编辑 → 经 CLI 重启 daemon。
/// 失败则回滚 active_id 并恢复旧 daemon。
fn set_active_profile_txn(
    app: &tauri::AppHandle,
    lifecycle: &lifecycle::Lifecycle,
    id: &str,
    skip_verify: bool,
    conn_edit: Option<&ConnectionEdit>,
) -> Result<serde_json::Value, String> {
    let dir = config::default_dir();
    let cfg = config::load_from(&dir).map_err(|e| e.to_string())?;
    let mut candidate = cfg
        .profile_by_id(id)
        .cloned()
        .ok_or_else(|| format!("找不到 profile：{id}"))?;

    let is_edit = conn_edit.is_some();
    if let Some(edit) = conn_edit {
        edit.apply(&mut candidate);
    }

    let (verb, tail): (&str, &str) = if is_edit {
        ("未保存", "仍在用原配置运行")
    } else {
        ("未切换", "当前配置不变")
    };

    assert_format_supported(&candidate)?;
    let launch = proxy_args_for(&candidate);
    if launch.key.is_empty() {
        return Err(format!("「{}」还没填 API key，请先填写。", candidate.name));
    }
    let native = is_native_adapter(&launch.adapter);
    if !native && launch.base_url.is_empty() {
        return Err("该配置需要填 base_url（http:// 或 https:// 开头）。".into());
    }
    if relay_missing_model(&launch.adapter, &candidate.model) {
        return Err("该配置需要选择或填写一个模型（中转/自定义端点必填），请在连接编辑里补上。".into());
    }

    let old_active = cfg.active_id.clone();

    // 1) scratch 校验候选
    let scratch_ok = if skip_scratch_verify(native, skip_verify) {
        true
    } else {
        let root = asset_root(app).ok_or("找不到代理脚本 proxy/csswitch_proxy.py。")?;
        let py = proc::find_exe("python3").ok_or("缺少依赖 python3（起临时代理需要）。")?;
        let script = root.join("proxy/csswitch_proxy.py");
        let res = scratch::scratch_probe(
            &py,
            &script,
            &scratch::ScratchTarget {
                provider: &launch.adapter,
                key_env: launch.key_env,
                base_url: &launch.base_url,
                key: &launch.key,
                model: Some(&launch.model),
                relay_thinking: launch.thinking_policy,
            },
            probe_kind_for(&launch.adapter, &launch.model),
        );
        match scratch::classify(res.status) {
            scratch::ProbeOutcome::Ok => true,
            scratch::ProbeOutcome::Auth(code) => {
                return Ok(json!({ "committed": false,
                    "hint": format!("上游拒绝（{code}），key/权限有误，{verb}（{tail}）。") }));
            }
            scratch::ProbeOutcome::ModelError(code) => {
                return Ok(json!({ "committed": false,
                    "hint": format!("上游拒绝该模型（{code}），{verb}。请换一个模型或核对 base_url。") }));
            }
            scratch::ProbeOutcome::Ambiguous(_)
            | scratch::ProbeOutcome::NoResponse
            | scratch::ProbeOutcome::Unsupported(_) => {
                return Ok(json!({ "committed": false, "can_skip": true,
                    "hint": format!("无法确认（网络/上游繁忙），{verb}。可重试，或用「跳过验证」。") }));
            }
        }
    };

    if !scratch_ok {
        return if is_edit {
            Err("连接上游校验失败（key/base_url/网络？），连接未保存。".into())
        } else {
            Err("候选上游校验失败（key/base_url/网络？），未切换。".into())
        };
    }

    // 2) 更新 config：active_id + 连接编辑
    lifecycle.bump_generation();
    if is_edit {
        config::write_rolling_backup(&dir).ok();
    }
    if let Err(e) = config::update(&dir, |c| {
        c.active_id = id.to_string();
        if let Some(edit) = conn_edit {
            if let Some(p) = c.profile_by_id_mut(id) {
                edit.apply(p);
            }
        }
    }) {
        return Err(format!("写盘失败（{e}），配置未更改。请检查磁盘空间/权限后重试。"));
    }

    // 3) 经 CLI 重启 daemon（先停旧、再起新）
    let _ = Command::new("csswitch")
        .args(["daemon", "stop"])
        .output();
    let output = Command::new("csswitch")
        .args(["daemon", "start"])
        .output()
        .map_err(|e| format!("执行 csswitch daemon start 失败：{e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        // 回滚 active_id
        let _ = config::update(&dir, |c| {
            c.active_id = old_active.clone();
        });
        // 尝试恢复旧 daemon
        let _ = Command::new("csswitch")
            .args(["daemon", "start"])
            .output();
        let msg = if is_edit {
            format!("校验通过，但 daemon 重启失败：{}，已回滚到原配置。", stderr.trim())
        } else {
            format!("校验通过，但 daemon 启动失败：{}，已回滚到原配置。", stderr.trim())
        };
        return Err(msg);
    }

    let hint = if is_edit {
        format!("已保存并应用「{}」的新连接。", candidate.name)
    } else {
        format!("已切到「{}」。", candidate.name)
    };
    Ok(json!({ "committed": true, "active_id": id, "hint": hint }))
}

#[tauri::command]
fn set_active_profile(
    app: tauri::AppHandle,
    lifecycle: State<'_, lifecycle::Lifecycle>,
    id: String,
    skip_verify: bool,
) -> Result<serde_json::Value, String> {
    lifecycle.with_serialized(|| {
        set_active_profile_txn(&app, lifecycle.inner(), &id, skip_verify, None)
    })
}

// ---------- 代理 / daemon（委托 CLI）----------

/// 启动 daemon：委托 `csswitch daemon start`。
#[tauri::command]
fn start_proxy() -> Result<serde_json::Value, String> {
    let output = Command::new("csswitch")
        .args(["daemon", "start"])
        .output()
        .map_err(|e| format!("执行 csswitch daemon start 失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!("csswitch daemon start 失败：{stderr}"));
    }
    let port: u16 = stdout
        .lines()
        .find(|l| l.contains("端口"))
        .and_then(|l| l.split_whitespace().find_map(|w| w.trim_end_matches(')').parse().ok()))
        .unwrap_or(18991);
    Ok(json!({ "port": port, "msg": stdout.trim().to_string() }))
}

/// 验证 key：确保 daemon 在跑，经代理向上游发最小请求。
#[tauri::command]
fn verify_key() -> Result<serde_json::Value, String> {
    let (port, secret, _action) = ensure_daemon_via_cli()?;
    let body = br#"{"model":"claude-opus-4-8","max_tokens":1,"messages":[{"role":"user","content":"ping"}]}"#;
    match proc::http_post_status(port, Some(&secret), "/v1/messages", body, 15000) {
        Some(200) => Ok(json!({ "ok": true, "hint": "key 有效，上游已接受。" })),
        Some(code @ (401 | 403)) => Ok(
            json!({ "ok": false, "hint": format!("上游拒绝（{code}），key 可能无效或无权限。") }),
        ),
        Some(code) => Ok(json!({
            "ok": false,
            "hint": format!("上游返回 {code}，可能是 key 无效、额度不足或上游异常。")
        })),
        None => Err("验证请求无响应（多为网络或上游不通）。".to_string()),
    }
}

#[derive(Deserialize)]
struct FetchModelsReq {
    #[serde(default)]
    template_id: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    profile_id: Option<String>,
}

#[tauri::command]
fn fetch_models(app: tauri::AppHandle, req: FetchModelsReq) -> Result<serde_json::Value, String> {
    let tid = req.template_id.trim();
    let tpl = templates::by_id(tid).ok_or_else(|| format!("未知模板：{tid}"))?;
    let base_url = if tpl.base_url_editable {
        req.base_url.trim().to_string()
    } else {
        tpl.base_url.to_string()
    };
    if base_url.is_empty() || !(base_url.starts_with("http://") || base_url.starts_with("https://"))
    {
        return Err("请先填写 base_url（http:// 或 https:// 开头）。".into());
    }
    let key = resolve_probe_key(req.profile_id.as_deref(), &req.key)?;
    let root = asset_root(&app).ok_or("找不到代理脚本 proxy/csswitch_proxy.py。")?;
    let py = proc::find_exe("python3").ok_or("缺少依赖 python3（起临时代理需要）。")?;
    let script = root.join("proxy/csswitch_proxy.py");

    let res = scratch::scratch_probe(
        &py,
        &script,
        &scratch::ScratchTarget {
            provider: "relay",
            key_env: "CSSWITCH_RELAY_KEY",
            base_url: &base_url,
            key: &key,
            model: None,
            relay_thinking: tpl.thinking_policy,
        },
        scratch::ProbeKind::Models,
    );
    let builtin = tpl.builtin_models;
    match scratch::classify(res.status) {
        scratch::ProbeOutcome::Ok => {
            let v: serde_json::Value =
                serde_json::from_str(&res.body).map_err(|e| format!("解析模型列表失败：{e}"))?;
            let live: Vec<(String, Option<bool>)> = v
                .get("data")
                .and_then(|d| d.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            let id = m.get("id")?.as_str()?.to_string();
                            let st = m.get("supports_tools").and_then(|b| b.as_bool());
                            Some((id, st))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({
                "models": merge_and_sort_models(live, builtin),
                "source": "live", "error_kind": null, "upstream_status": 200
            }))
        }
        scratch::ProbeOutcome::Auth(code) => {
            Err(format!("上游拒绝（{code}），key 或权限可能有误。"))
        }
        other => {
            let source = scratch::discovery_fallback_source(&other);
            let error_kind = if source == "network" {
                json!("network")
            } else {
                json!(null)
            };
            Ok(json!({
                "models": merge_and_sort_models(vec![], builtin),
                "source": source,
                "error_kind": error_kind,
                "upstream_status": res.status
            }))
        }
    }
}

/// 停止 daemon + 刷新托盘图标。
#[tauri::command]
fn stop_all(app: tauri::AppHandle) -> Result<(), String> {
    let _ = Command::new("csswitch").args(["daemon", "stop"]).output();
    tray::refresh_tray_icon(&app);
    Ok(())
}

// ---------- 一键代理（委托 CLI daemon + 沙箱管理）----------

#[tauri::command]
fn one_click_login(
    app: tauri::AppHandle,
    state: State<'_, Mutex<AppState>>,
    lifecycle: State<'_, lifecycle::Lifecycle>,
) -> Result<serde_json::Value, String> {
    lifecycle.with_serialized(|| one_click_login_inner(app, state))
}

/// 一键代理：确保 CLI daemon 在跑 → 虚拟 OAuth 登录 → 起沙箱 → 打开浏览器。
/// **Desktop 不自己 spawn 代理**，全委托 `csswitch daemon start`。
fn one_click_login_inner(
    app: tauri::AppHandle,
    state: State<'_, Mutex<AppState>>,
) -> Result<serde_json::Value, String> {
    // 1. 确保 CLI daemon 在跑（幂等，已健康则复用）
    let (pport, secret, daemon_action) = ensure_daemon_via_cli()?;

    let dir = config::default_dir();
    let cfg = config::load_from(&dir).map_err(|e| e.to_string())?;
    let sport = cfg.sandbox_port;

    let sbx_home = sandbox_home();
    let auth_dir = sbx_home.join(".claude-science");

    // 沙箱已健康且登录完好 → 直接打开
    if sandbox_running_ours(sport) {
        if oauth_forge::login_intact(&auth_dir, "virtual@localhost.invalid", &sbx_home) {
            let url = sandbox_url(sport);
            {
                let mut st = lock(&state);
                st.sandbox_port = sport;
                st.sandbox_url = Some(url.clone());
            }
            let msg = match open_in_browser(&url) {
                Ok(()) => format!("代理{daemon_action}，已重新打开 Science。"),
                Err(_) => format!("代理{daemon_action}，服务已就绪，请手动打开：{url}"),
            };
            return Ok(json!({ "url": url, "msg": msg, "action": "reopened" }));
        }
        {
            let mut st = lock(&state);
            let _ = stop_sandbox_inner(&app, &mut st);
        }
    }

    // 2. 确保虚拟登录（幂等）
    let root = asset_root(&app)
        .ok_or("找不到 scripts/launch-virtual-sandbox.sh（打包资源或仓库根均未命中）。")?;

    let (forged, login_action) =
        oauth_forge::ensure_virtual_login(&auth_dir, "virtual@localhost.invalid", &sbx_home)
            .map_err(|e| format!("写虚拟登录失败：{e}"))?;

    // 3. 起沙箱
    let launch = root.join("scripts/launch-virtual-sandbox.sh");
    if !launch.is_file() {
        return Err("找不到 scripts/launch-virtual-sandbox.sh。".into());
    }

    let proxy_url = format!("http://127.0.0.1:{pport}/{secret}");
    let logf = open_log("sandbox.log").map_err(|e| format!("建日志失败：{e}"))?;
    {
        use std::io::Write;
        let mut lw = &logf;
        let _ = writeln!(
            lw,
            "[oauth] 虚拟登录已就绪（Rust，零 node；action={:?}）：auth_dir={} account={} org={} enc={}",
            login_action,
            forged.auth_dir.display(),
            forged.account_uuid,
            forged.org_uuid,
            forged.enc_file.display()
        );
    }
    let logf2 = logf.try_clone().map_err(|e| e.to_string())?;
    let shell = system_shell();
    let status = Command::new(&shell)
        .arg(&launch)
        .arg("--port")
        .arg(sport.to_string())
        .arg("--proxy-url")
        .arg(&proxy_url)
        .arg("--skip-oauth-forge")
        .env("SANDBOX_HOME", sandbox_home())
        .stdout(Stdio::from(logf))
        .stderr(Stdio::from(logf2))
        .status()
        .map_err(|e| format!("起沙箱失败：{e}"))?;
    if !status.success() {
        let tail = redact(&tail_file(&log_path("sandbox.log"), 600), &secret);
        return Err(format!("起沙箱脚本失败。\n{tail}"));
    }

    // 4. 轮询沙箱 /health
    let mut ok = false;
    for _ in 0..80 {
        std::thread::sleep(Duration::from_millis(100));
        if proc::http_health(sport, None, 400) {
            ok = true;
            break;
        }
    }
    if !ok {
        let tail = redact(&tail_file(&log_path("sandbox.log"), 600), &secret);
        {
            let mut st = lock(&state);
            let _ = stop_sandbox_inner(&app, &mut st);
        }
        return Err(format!(
            "沙箱起后探活超时（端口 {sport}）。已尝试停掉刚起的沙箱。\n{tail}"
        ));
    }

    if !sandbox_running_ours(sport) {
        {
            let mut st = lock(&state);
            let _ = stop_sandbox_inner(&app, &mut st);
        }
        return Err(format!(
            "端口 {sport} 有服务响应，但按 data-dir 确认不是本沙箱 Science（疑似被其它服务占用）。已尝试停掉刚起的沙箱。"
        ));
    }

    // 5. 打开浏览器
    let url = sandbox_url(sport);
    {
        let mut st = lock(&state);
        st.sandbox_port = sport;
        st.sandbox_url = Some(url.clone());
    }
    let started = match login_action {
        oauth_forge::LoginAction::Created => "已启动",
        _ => "沙箱已重新启动，沿用原有对话",
    };
    let msg = match open_in_browser(&url) {
        Ok(()) => format!("{started}。"),
        Err(_) => format!("{started}，服务已就绪，请手动打开：{url}"),
    };
    Ok(json!({ "url": url, "msg": msg, "action": "started" }))
}

// ---------- 状态 / 辅助命令 ----------

/// 系统状态：daemon（经 CLI status）+ 沙箱（直接探活）+ 上游（TCP 可达性）。
#[tauri::command]
fn status() -> serde_json::Value {
    let dir = config::default_dir();
    let cfg = config::load_from(&dir).unwrap_or_default();
    let pport = cfg.proxy_port;
    let sport = cfg.sandbox_port;
    let secret = cfg.secret.clone();

    // daemon 状态：优先 CLI，回退直接探活
    let proxy = if !secret.is_empty() && proc::http_health(pport, Some(&secret), 300) {
        "green"
    } else {
        "amber"
    };
    let sandbox = if sandbox_running_ours(sport) {
        "green"
    } else {
        "amber"
    };
    let (adapter, base_url) = match cfg.active_profile() {
        Some(p) => (
            templates::adapter_for(&p.template_id).to_string(),
            p.base_url.clone(),
        ),
        None => (String::new(), String::new()),
    };
    let uhost = upstream_host(&adapter, &base_url);
    let upstream = if !uhost.is_empty() && proc::tcp_reachable(&uhost, 443, 500) {
        "green"
    } else {
        "amber"
    };
    json!({ "proxy": proxy, "sandbox": sandbox, "upstream": upstream })
}

#[tauri::command]
fn open_url(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let url = { lock(&state).sandbox_url.clone() };
    let url = url.ok_or("还没有沙箱 URL，请先「一键代理」。")?;
    open_in_browser(&url)
}

#[tauri::command]
fn run_doctor(app: tauri::AppHandle) -> Result<String, String> {
    let root = asset_root(&app).ok_or("找不到 scripts/doctor.sh（打包资源或仓库根均未命中）。")?;
    let cfg = config::load_from(&config::default_dir()).unwrap_or_default();
    let doctor = root.join("scripts/doctor.sh");
    let (provider_label, adapter, has_key) = match cfg.active_profile() {
        Some(p) => (
            p.template_id.clone(),
            templates::adapter_for(&p.template_id),
            !p.api_key.is_empty(),
        ),
        None => (String::new(), "", false),
    };
    let mut cmd = Command::new("bash");
    cmd.arg(&doctor)
        .env("CSSWITCH_PROVIDER", &provider_label)
        .env("CSSWITCH_ADAPTER", adapter)
        .env("CSSWITCH_KEY_PRESENT", if has_key { "1" } else { "0" })
        .env("CSSWITCH_PROXY_PORT", cfg.proxy_port.to_string())
        .env("CSSWITCH_SANDBOX_PORT", cfg.sandbox_port.to_string());
    let out = cmd.output().map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        text.push_str("\n[stderr] ");
        text.push_str(err.trim());
    }
    Ok(text)
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
fn open_release_page() -> Result<(), String> {
    open_in_browser("https://github.com/SuperJJ007/CSSwitch/releases/latest")
}

#[tauri::command]
fn report_bug() -> Result<(), String> {
    open_in_browser("https://github.com/SuperJJ007/CSSwitch/issues/new?template=bug_report.yml")
}

#[tauri::command]
fn open_logs() -> Result<(), String> {
    let dir = config::default_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(opener)
        .arg(&dir)
        .status()
        .map_err(|e| format!("打开日志目录失败：{e}"))?;
    Ok(())
}

/// 退出 GUI。daemon 独立运行，不杀。托盘「终止代理并退出」走 stop_all 而非此命令。
#[tauri::command]
fn quit_app(app: tauri::AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

// ---------- 入口 ----------
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Mutex::new(AppState::default()))
        .manage(lifecycle::Lifecycle::new())
        .invoke_handler(tauri::generate_handler![
            get_config,
            list_templates,
            set_settings,
            set_mode,
            open_official,
            create_profile,
            update_profile_metadata,
            update_profile_connection,
            clear_profile_key,
            delete_profile,
            set_active_profile,
            start_proxy,
            verify_key,
            fetch_models,
            stop_all,
            one_click_login,
            status,
            open_url,
            run_doctor,
            app_version,
            open_release_page,
            report_bug,
            open_logs,
            quit_app,
            tray::update_tray_icon
        ])
        .setup(|app| {
            // 启动时检查 daemon 状态，刷新托盘图标
            let _ = config::load_from(&config::default_dir());

            // 关闭窗口 = 隐藏到托盘（daemon 独立常驻，不受 GUI 生命周期影响）
            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                win.on_window_event(move |ev| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            tray::setup_tray(app)?;
            tray::refresh_tray_icon(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ========== Tests ==========

#[cfg(test)]
mod tests {
    use super::{
        assert_format_supported, build_get_config, build_list_templates, clear_profile_key_inner,
        create_profile_inner, delete_profile_inner, first_http_url, is_main_list_model,
        key_env_for_adapter, merge_and_sort_models, nonactive_probe_verdict, parse_host,
        probe_kind_for, probe_kind_for_model, proxy_args_for, redact, relay_missing_base_url,
        relay_missing_model, sandbox_home, settings_change_needs_teardown, should_scratch_candidate,
        skip_scratch_verify, update_profile_metadata_inner, upstream_host, ConnectionEdit,
    };
    use crate::config;

    fn tmpdir_lib() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("csswitch-lib-test-{}", std::process::id()));
        let d = base.join(format!(
            "{:?}-{}",
            std::thread::current().id(),
            config::new_id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join(".csswitch")
    }

    #[test]
    fn proxy_args_derive_adapter_and_key_env() {
        use crate::config::Profile;
        let ds = Profile {
            template_id: "deepseek".into(),
            api_format: "anthropic".into(),
            base_url: "https://api.deepseek.com/anthropic".into(),
            api_key: "sk-ds".into(),
            ..Default::default()
        };
        let a = proxy_args_for(&ds);
        assert_eq!(a.adapter, "deepseek");
        assert_eq!(a.key_env, "DEEPSEEK_API_KEY");

        let glm = Profile {
            template_id: "glm".into(),
            api_format: "anthropic".into(),
            base_url: "https://open.bigmodel.cn/api/anthropic".into(),
            api_key: "gk".into(),
            model: "glm-5".into(),
            ..Default::default()
        };
        let b = proxy_args_for(&glm);
        assert_eq!(b.adapter, "relay");
        assert_eq!(b.key_env, "CSSWITCH_RELAY_KEY");
    }

    #[test]
    fn unsupported_api_format_is_rejected() {
        use crate::config::Profile;
        let p = Profile {
            template_id: "custom".into(),
            api_format: "gemini_native".into(),
            base_url: "https://x/y".into(),
            api_key: "k".into(),
            ..Default::default()
        };
        assert!(assert_format_supported(&p).is_err());
    }

    #[test]
    fn key_env_for_adapter_maps_adapters() {
        assert_eq!(key_env_for_adapter("deepseek"), "DEEPSEEK_API_KEY");
        assert_eq!(key_env_for_adapter("qwen"), "DASHSCOPE_API_KEY");
        assert_eq!(key_env_for_adapter("relay"), "CSSWITCH_RELAY_KEY");
    }

    #[test]
    fn settings_teardown_when_any_port_changes() {
        assert!(!settings_change_needs_teardown(18991, 18991, 8990, 8990));
        assert!(settings_change_needs_teardown(18991, 19000, 8990, 8990));
        assert!(settings_change_needs_teardown(18991, 18991, 8990, 9000));
        assert!(settings_change_needs_teardown(18991, 19000, 8990, 9000));
    }

    #[test]
    fn nonactive_probe_verdict_maps_outcomes() {
        use crate::scratch::ProbeOutcome;
        assert!(nonactive_probe_verdict(&ProbeOutcome::Auth(401))
            .unwrap_err()
            .contains("401"));
        assert!(nonactive_probe_verdict(&ProbeOutcome::ModelError(404))
            .unwrap_err()
            .contains("404"));
        assert_eq!(nonactive_probe_verdict(&ProbeOutcome::Ok), Ok(true));
        assert_eq!(
            nonactive_probe_verdict(&ProbeOutcome::Ambiguous(Some(429))),
            Ok(false)
        );
        assert_eq!(
            nonactive_probe_verdict(&ProbeOutcome::NoResponse),
            Ok(false)
        );
    }

    #[test]
    fn connection_edit_apply_only_changes_provided_fields() {
        use crate::config::Profile;
        let mut p = Profile {
            base_url: "old-url".into(),
            api_format: "anthropic".into(),
            model: "old-model".into(),
            api_key: "old-key".into(),
            ..Default::default()
        };
        let edit = ConnectionEdit {
            base_url: Some("new-url".into()),
            api_format: None,
            model: Some("new-model".into()),
            key: Some(String::new()),
        };
        edit.apply(&mut p);
        assert_eq!(p.base_url, "new-url");
        assert_eq!(p.api_format, "anthropic");
        assert_eq!(p.model, "new-model");
        assert_eq!(p.api_key, "old-key");

        let edit2 = ConnectionEdit {
            key: Some("new-key".into()),
            ..Default::default()
        };
        edit2.apply(&mut p);
        assert_eq!(p.api_key, "new-key");
    }

    #[test]
    fn create_profile_from_template_prefills() {
        let d = tmpdir_lib();
        let id =
            create_profile_inner(&d, "glm", "GLM", Some("gk"), None, Some("glm-5.2")).unwrap();
        let cfg = config::load_from(&d).unwrap();
        let p = cfg.profile_by_id(&id).unwrap();
        assert_eq!(p.template_id, "glm");
        assert_eq!(p.api_format, "anthropic");
        assert_eq!(p.base_url, "https://open.bigmodel.cn/api/anthropic");
    }

    #[test]
    fn create_relay_without_model_is_rejected() {
        let d = tmpdir_lib();
        let e = create_profile_inner(&d, "glm", "GLM", Some("gk"), None, None);
        assert!(e.is_err());
        assert!(create_profile_inner(&d, "deepseek", "DS", Some("gk"), None, None).is_ok());
    }

    #[test]
    fn update_metadata_does_not_touch_key() {
        let d = tmpdir_lib();
        let id =
            create_profile_inner(&d, "glm", "GLM", Some("secret9"), None, Some("glm-5.2")).unwrap();
        update_profile_metadata_inner(&d, &id, "renamed", Some("note")).unwrap();
        let cfg = config::load_from(&d).unwrap();
        let p = cfg.profile_by_id(&id).unwrap();
        assert_eq!(p.name, "renamed");
        assert_eq!(p.api_key, "secret9");
    }

    #[test]
    fn clear_key_empties_key_and_drops_backup() {
        let d = tmpdir_lib();
        let id =
            create_profile_inner(&d, "glm", "GLM", Some("secretTAIL"), None, Some("glm-5.2"))
                .unwrap();
        config::write_rolling_backup(&d).ok();
        clear_profile_key_inner(&d, &id).unwrap();
        let cfg = config::load_from(&d).unwrap();
        assert_eq!(cfg.profile_by_id(&id).unwrap().api_key, "");
        assert!(!d.join("config.json.bak").exists());
    }

    #[test]
    fn delete_active_clears_active() {
        let d = tmpdir_lib();
        let id =
            create_profile_inner(&d, "glm", "GLM", Some("k"), None, Some("glm-5.2")).unwrap();
        config::update(&d, |c| c.active_id = id.clone()).unwrap();
        delete_profile_inner(&d, &id).unwrap();
        let cfg = config::load_from(&d).unwrap();
        assert!(cfg.profile_by_id(&id).is_none());
        assert_eq!(cfg.active_id, "");
    }

    #[test]
    fn update_metadata_unknown_id_errors() {
        let d = tmpdir_lib();
        create_profile_inner(&d, "glm", "GLM", Some("k"), None, Some("glm-5.2")).unwrap();
        assert!(update_profile_metadata_inner(&d, "no-such-id", "x", None).is_err());
    }

    #[test]
    fn get_config_masks_keys_and_lists_profiles() {
        let d = tmpdir_lib();
        let id = create_profile_inner(
            &d, "glm", "GLM", Some("sk-longsecret9999"), None, Some("glm-5.2"),
        )
        .unwrap();
        let v = build_get_config(&d).unwrap();
        let arr = v["profiles"].as_array().unwrap();
        let p = arr.iter().find(|p| p["id"] == id).unwrap();
        assert!(p["key"].as_str().unwrap().ends_with("9999"));
        assert!(!p["key"].as_str().unwrap().contains("longsecret"));
    }

    #[test]
    fn list_templates_has_nine() {
        let v = build_list_templates();
        assert_eq!(v.len(), 9);
    }

    #[test]
    fn first_http_url_takes_only_first_valid_url() {
        assert_eq!(
            first_http_url("http://127.0.0.1:8990/setup?nonce=abc").as_deref(),
            Some("http://127.0.0.1:8990/setup?nonce=abc"),
        );
        assert_eq!(first_http_url("no url here"), None);
    }

    #[test]
    fn parse_host_extracts_host() {
        assert_eq!(
            parse_host("https://byteswarm.ai/claude").as_deref(),
            Some("byteswarm.ai")
        );
        assert_eq!(parse_host("byteswarm.ai/claude"), None);
    }

    #[test]
    fn upstream_host_by_adapter() {
        assert_eq!(upstream_host("deepseek", ""), "api.deepseek.com");
        assert_eq!(upstream_host("qwen", ""), "dashscope.aliyuncs.com");
        assert_eq!(
            upstream_host("relay", "https://open.bigmodel.cn/api/anthropic"),
            "open.bigmodel.cn"
        );
    }

    #[test]
    fn main_list_model_matches_family_plus_digit() {
        assert!(is_main_list_model("claude-opus-4-8"));
        assert!(is_main_list_model("claude-sonnet-5"));
        assert!(!is_main_list_model("claude-fable-5"));
        assert!(!is_main_list_model("gpt-4o"));
    }

    #[test]
    fn redact_scrubs_secret() {
        assert!(!redact("leak abcd1234 leak", "abcd1234").contains("abcd1234"));
        assert_eq!(redact("safe", ""), "safe");
    }

    #[test]
    fn sandbox_home_is_under_config_dir() {
        let h = sandbox_home();
        assert!(h.ends_with("sandbox/home"));
        assert!(h.to_string_lossy().contains(".csswitch"));
    }

    #[test]
    fn merge_and_sort_prefers_tools_then_dedupes() {
        let live = vec![
            ("m-notools".to_string(), Some(false)),
            ("m-tools".to_string(), Some(true)),
        ];
        let out = merge_and_sort_models(live, &["m-tools", "m-builtin-only"]);
        let ids: Vec<String> = out.iter().map(|v| v["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids[0], "m-tools");
        assert!(ids.contains(&"m-builtin-only".to_string()));
        assert_eq!(ids.iter().filter(|i| *i == "m-tools").count(), 1);
    }

    #[test]
    fn probe_kind_picks_message_when_model_set() {
        assert!(matches!(probe_kind_for_model("mimo-v2.5-pro"), crate::scratch::ProbeKind::Message));
        assert!(matches!(probe_kind_for_model(""), crate::scratch::ProbeKind::Models));
    }

    #[test]
    fn native_probe_uses_message() {
        assert!(matches!(probe_kind_for("deepseek", ""), crate::scratch::ProbeKind::Message));
        assert!(matches!(probe_kind_for("qwen", ""), crate::scratch::ProbeKind::Message));
        assert!(matches!(probe_kind_for("relay", ""), crate::scratch::ProbeKind::Models));
        assert!(matches!(probe_kind_for("relay", "m1"), crate::scratch::ProbeKind::Message));
    }

    #[test]
    fn native_adapter_no_longer_bypasses_verify() {
        assert!(!skip_scratch_verify(true, false));
        assert!(!skip_scratch_verify(false, false));
        assert!(skip_scratch_verify(false, true));
    }

    #[test]
    fn native_candidate_is_upstream_validated_even_without_base_url() {
        assert!(should_scratch_candidate("deepseek", "sk-x", ""));
        assert!(should_scratch_candidate("qwen", "sk-x", ""));
        assert!(!should_scratch_candidate("relay", "sk-x", ""));
        assert!(!should_scratch_candidate("deepseek", "", ""));
    }

    #[test]
    fn relay_empty_base_url_is_rejected() {
        assert!(relay_missing_base_url("relay", ""));
        assert!(relay_missing_base_url("custom", ""));
        assert!(!relay_missing_base_url("relay", "https://r"));
        assert!(!relay_missing_base_url("deepseek", ""));
    }

    #[test]
    fn relay_empty_model_is_rejected() {
        assert!(relay_missing_model("relay", ""));
        assert!(relay_missing_model("custom", ""));
        assert!(!relay_missing_model("relay", "glm-5.2"));
        assert!(!relay_missing_model("deepseek", ""));
    }
}
