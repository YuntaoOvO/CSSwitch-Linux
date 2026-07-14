//! CSSwitch CLI – provider switcher and launcher for Claude Science.
//!
//! Usage:
//!   csswitch profile list
//!   csswitch profile add --template deepseek --name "DS" --key sk-xxx
//!   csswitch profile activate <id>
//!   csswitch proxy start|stop|status
//!   csswitch science start|stop|status
//!   csswitch daemon start|stop|status
//!   csswitch run -- <command> [args...]
//!   csswitch hook install|uninstall [--shell bash|zsh|fish]
//!   csswitch env
//!   csswitch doctor
//!   csswitch config
//!   csswitch --help

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use csswitch_runtime::{
    self,
    operation::{OperationKind, OperationTrace, OperationStage},
    proxy_lifecycle::{ensure_proxy, find_gateway_bin},
    proxy::ProxyAction,
    science::{
        find_science_bin, proxy_env_vars, sandbox_health, sandbox_url,
        start_science_sandbox,
    },
    system::{daemon_pid_file, repo_root, sandbox_home, append_operation_log},
    AppState, RuntimeContext,
};
use csswitch_config::{self, Profile};

// ── RuntimeContext for CLI ──

struct CliContext;

impl RuntimeContext for CliContext {
    fn asset_root(&self) -> Option<PathBuf> {
        repo_root().map(|r| r.join("scripts"))
    }

    fn repo_root(&self) -> Option<PathBuf> {
        repo_root()
    }

    fn log_dir(&self) -> PathBuf {
        csswitch_config::default_dir().join("logs")
    }

    fn open_browser(&self, url: &str) -> Result<(), String> {
        let cmd = if cfg!(target_os = "linux") {
            if Command::new("xdg-open").arg("--version").output().is_ok() {
                "xdg-open"
            } else {
                "sensible-browser"
            }
        } else {
            "open"
        };
        let st = Command::new(cmd)
            .arg(url)
            .status()
            .map_err(|e| format!("打开浏览器失败：{e}"))?;
        if !st.success() {
            return Err(format!("{} 非零退出", cmd));
        }
        Ok(())
    }

    fn append_operation_log(&self, line: &str) {
        append_operation_log(line);
    }
}

// ── Simple argument parser ──

struct Args {
    command: String,
    subcommand: String,
    flags: Vec<(String, Option<String>)>,
    positional: Vec<String>,
    rest: Vec<String>, // after --
}

fn parse_args() -> Args {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut command = String::new();
    let mut subcommand = String::new();
    let mut flags: Vec<(String, Option<String>)> = Vec::new();
    let mut positional: Vec<String> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    let mut in_rest = false;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            in_rest = true;
            i += 1;
            continue;
        }
        if in_rest {
            rest.push(arg.clone());
            i += 1;
            continue;
        }
        if arg.starts_with("--") {
            let flag = arg[2..].to_string();
            if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags.push((flag, Some(args[i + 1].clone())));
                i += 2;
            } else {
                flags.push((flag, None));
                i += 1;
            }
        } else if command.is_empty() {
            command = arg.clone();
            i += 1;
        } else if subcommand.is_empty() && !arg.starts_with("-") {
            subcommand = arg.clone();
            i += 1;
        } else {
            positional.push(arg.clone());
            i += 1;
        }
    }

    Args { command, subcommand, flags, positional, rest }
}

fn flag_value(flags: &[(String, Option<String>)], name: &str) -> Option<String> {
    flags.iter().find(|(f, _)| f == name).and_then(|(_, v)| v.clone())
}

fn has_flag(flags: &[(String, Option<String>)], name: &str) -> bool {
    flags.iter().any(|(f, _)| f == name)
}

fn print_usage() {
    println!(r#"CSSwitch CLI v0.5.0 – provider switcher for Claude Science

USAGE:
  csswitch <command> [subcommand] [flags] [args]

COMMANDS:
  profile   Manage provider profiles
    list              List all profiles
    add               Add a new profile
      --template <id>   Template (deepseek, qwen, glm, kimi, siliconflow, ...)
      --name <name>     Display name
      --key <apikey>    API key
      --base-url <url>  Base URL override (optional)
      --model <model>   Model override (optional)
    delete <id>       Delete a profile
    activate <id>     Set as active profile
    show [id]         Show profile details

  proxy     Control proxy gateway
    start             Start proxy
    stop              Stop proxy
    status            Show proxy status

  science   Control Claude Science sandbox
    start             Start sandbox (one-click)
    stop              Stop sandbox
    status            Show sandbox status

  daemon    Daemon lifecycle
    start             Start background daemon
    stop              Stop daemon
    status            Show daemon status

  run       Run command with proxy env
    -- <cmd> [args]    Execute command with ANTHROPIC_BASE_URL set

  hook      Shell hook management
    install           Install shell hook
      --shell <sh>      bash (default), zsh, fish
    uninstall         Remove shell hook

  env       Print proxy environment variables (for eval)

  doctor    Read-only environment diagnostics

  config    Show current configuration (keys redacted)

  --help    Show this help
"#);
}

// ── Main ──

fn main() {
    let args = parse_args();

    if args.command.is_empty() || has_flag(&args.flags, "help") {
        print_usage();
        return;
    }

    let ctx = CliContext;

    let result = match args.command.as_str() {
        "profile" => handle_profile(&args),
        "proxy" => handle_proxy(&ctx, &args),
        "science" => handle_science(&ctx, &args),
        "daemon" => handle_daemon(&ctx, &args),
        "run" => handle_run(&args),
        "hook" => handle_hook(&args),
        "env" => handle_env(),
        "doctor" => handle_doctor(),
        "config" => handle_config(),
        _ => {
            eprintln!("csswitch: 未知命令 '{}'，用 --help 查看帮助", args.command);
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("csswitch: {e}");
        std::process::exit(1);
    }
}

// ── Profile commands ──

fn handle_profile(args: &Args) -> Result<(), String> {
    match args.subcommand.as_str() {
        "list" => {
            let dir = csswitch_config::default_dir();
            let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
            if cfg.profiles.is_empty() {
                println!("(没有保存的配置)");
                return Ok(());
            }
            for p in &cfg.profiles {
                let active = if p.id == cfg.active_id { " *" } else { "  " };
                let key_status = if p.api_key.is_empty() { "(无key)" } else { "(有key)" };
                println!("{active}{}  {}  {}  {}", p.id, p.name, p.template_id, key_status);
            }
            Ok(())
        }
        "add" => {
            let template = flag_value(&args.flags, "template")
                .ok_or("--template 是必填参数（deepseek, qwen, glm, kimi, siliconflow, ...）")?;
            let name = flag_value(&args.flags, "name")
                .ok_or("--name 是必填参数")?;
            let key = flag_value(&args.flags, "key")
                .ok_or("--key 是必填参数")?;
            let base_url = flag_value(&args.flags, "base-url");
            let model = flag_value(&args.flags, "model");

            let t = csswitch_templates::by_id(&template)
                .ok_or_else(|| format!("未知模板：{template}。可用：deepseek, qwen, glm, kimi, siliconflow, xiaomi, openrouter, custom"))?;

            let id = format!("{:016x}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
            let profile = Profile {
                id: id.clone(),
                name,
                template_id: t.id.to_string(),
                category: t.category.to_string(),
                api_format: t.api_format.to_string(),
                base_url: base_url.unwrap_or_else(|| t.base_url.to_string()),
                api_key: key,
                model: model.unwrap_or_default(),
                website_url: Some(t.website_url.to_string()),
                icon: Some(t.icon.to_string()),
                icon_color: Some(t.icon_color.to_string()),
                ..Default::default()
            };

            let dir = csswitch_config::default_dir();
            csswitch_config::update(&dir, |cfg| {
                cfg.profiles.push(profile);
            }).map_err(|e| e.to_string())?;

            println!("已添加 profile：{id}");
            Ok(())
        }
        "delete" => {
            let id = args.positional.first()
                .ok_or("用法：csswitch profile delete <id>")?;
            let dir = csswitch_config::default_dir();
            csswitch_config::update(&dir, |cfg| {
                cfg.profiles.retain(|p| p.id != *id);
                if cfg.active_id == *id {
                    cfg.active_id.clear();
                }
            }).map_err(|e| e.to_string())?;
            println!("已删除 profile：{id}");
            Ok(())
        }
        "activate" => {
            let id = args.positional.first()
                .ok_or("用法：csswitch profile activate <id>")?;
            let dir = csswitch_config::default_dir();
            let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
            let profile = cfg.profile_by_id(id)
                .ok_or_else(|| format!("找不到 profile：{id}"))?
                .clone();

            if profile.api_key.is_empty() {
                return Err("该 profile 还没填 API key".to_string());
            }
            csswitch_runtime::provider::assert_format_supported(&profile)?;

            csswitch_config::update(&dir, |cfg| {
                cfg.active_id = id.clone();
            }).map_err(|e| e.to_string())?;

            println!("已激活 profile：{id} ({})", profile.name);
            Ok(())
        }
        "show" => {
            let dir = csswitch_config::default_dir();
            let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
            let profile = if let Some(id) = args.positional.first() {
                cfg.profile_by_id(id)
                    .ok_or_else(|| format!("找不到 profile：{id}"))?
            } else {
                cfg.active_profile()
                    .ok_or_else(|| "当前没有生效的 profile".to_string())?
            };
            let key_masked = csswitch_config::mask(&profile.api_key);
            println!("ID:        {}", profile.id);
            println!("名称:      {}", profile.name);
            println!("模板:      {}", profile.template_id);
            println!("类别:      {}", profile.category);
            println!("API格式:   {}", profile.api_format);
            println!("Base URL:  {}", profile.base_url);
            println!("模型:      {}", if profile.model.is_empty() { "(默认)" } else { &profile.model });
            println!("API Key:   {}", key_masked);
            Ok(())
        }
        _ => {
            eprintln!("csswitch profile: 未知子命令 '{}'", args.subcommand);
            Err("用 csswitch --help 查看帮助".to_string())
        }
    }
}

// ── Proxy commands ──

fn handle_proxy(ctx: &dyn RuntimeContext, args: &Args) -> Result<(), String> {
    let state = Arc::new(Mutex::new(AppState::default()));

    match args.subcommand.as_str() {
        "start" => {
            let dir = csswitch_config::default_dir();
            let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
            let trace = OperationTrace::start(OperationKind::StartProxy, "cli".to_string());
            let (secret, port, action) = ensure_proxy(ctx, &state, &cfg, 1)?;
            write_proxy_env_file(port, &secret)?;
            // Persist generated secret to config
            csswitch_config::update(&dir, |c| c.secret = secret.clone())
                .map_err(|e| e.to_string())?;
            let msg = match action {
                ProxyAction::Reused => "代理已在运行，复用中",
                ProxyAction::Restarted => "代理已启动",
            };
            println!("{msg} (端口 {port})");
            trace.finish("ok");
            Ok(())
        }
        "stop" => {
            let mut st = csswitch_runtime::lock(&state);
            st.stop_proxy();
            let _ = std::fs::remove_file(proxy_env_file_path());
            println!("代理已停止");
            Ok(())
        }
        "status" => {
            let st = csswitch_runtime::lock(&state);
            if st.secret.len() > 0 {
                let healthy = csswitch_runtime::proxy_lifecycle::proxy_health(st.proxy_port, &st.secret);
                if healthy {
                    println!("代理状态: 运行中 (端口 {}, provider: {})", st.proxy_port, st.provider);
                } else {
                    println!("代理状态: 进程存在但探活失败 (端口 {})", st.proxy_port);
                }
            } else {
                println!("代理状态: 未运行");
            }
            Ok(())
        }
        _ => Err("用法：csswitch proxy start|stop|status".to_string()),
    }
}

// ── Science commands ──

fn handle_science(ctx: &dyn RuntimeContext, args: &Args) -> Result<(), String> {
    let state = Arc::new(Mutex::new(AppState::default()));

    match args.subcommand.as_str() {
        "start" => {
            let dir = csswitch_config::default_dir();
            let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
            let sport = cfg.sandbox_port;
            let trace = OperationTrace::start(OperationKind::OneClickLogin, "cli".to_string());

            // Check if already running
            if sandbox_health(sport) {
                let url = sandbox_url(sport);
                println!("Science 沙箱已在运行：{url}");
                return Ok(());
            }

            // Ensure proxy
            let (secret, pport, _proxy_action) = ensure_proxy(ctx, &state, &cfg, 1)?;
            // Persist generated secret to config
            csswitch_config::update(&dir, |c| c.secret = secret.clone())
                .map_err(|e| e.to_string())?;
            trace.stage(OperationStage::ProxyHealth, "ready");

            // Ensure virtual login
            let sbx_home = sandbox_home();
            let auth_dir = sbx_home.join(".claude-science");
            csswitch_oauth::ensure_virtual_login(&auth_dir, "virtual@localhost.invalid", &sbx_home)
                .map_err(|e| format!("写虚拟登录失败：{e}"))?;

            // Start science sandbox
            let child = start_science_sandbox(sport, pport, &secret, Some(&trace))?;
            {
                let mut st = csswitch_runtime::lock(&state);
                st.sandbox = Some(child);
                st.sandbox_port = sport;
                st.sandbox_url = Some(sandbox_url(sport));
            }

            let url = sandbox_url(sport);
            println!("Science 沙箱已启动：{url}");
            trace.finish("ok");

            if let Err(e) = ctx.open_browser(&url) {
                println!("(无法自动打开浏览器：{e})");
            }
            Ok(())
        }
        "stop" => {
            let mut st = csswitch_runtime::lock(&state);
            st.stop_sandbox();
            println!("Science 沙箱已停止");
            Ok(())
        }
        "status" => {
            let dir = csswitch_config::default_dir();
            if let Ok(cfg) = csswitch_config::load_from(&dir) {
                if sandbox_health(cfg.sandbox_port) {
                    println!("Science 沙箱: 运行中 ({})", sandbox_url(cfg.sandbox_port));
                } else {
                    println!("Science 沙箱: 未运行");
                }
            } else {
                println!("Science 沙箱: 无法读取配置");
            }
            Ok(())
        }
        _ => Err("用法：csswitch science start|stop|status".to_string()),
    }
}

// ── Run command ──

fn handle_run(args: &Args) -> Result<(), String> {
    let cmd_args = if !args.rest.is_empty() {
        &args.rest
    } else {
        &args.positional
    };

    if cmd_args.is_empty() {
        return Err("用法：csswitch run -- <命令> [参数...]".to_string());
    }

    let dir = csswitch_config::default_dir();
    let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
    cfg.active_profile()
        .ok_or_else(|| "当前没有生效的 profile，请先用 `csswitch profile activate <id>` 设置。".to_string())?;

    let port = cfg.proxy_port;
    let secret = if cfg.secret.is_empty() {
        return Err("代理尚未启动，请先运行 `csswitch proxy start`".to_string());
    } else {
        cfg.secret.clone()
    };

    // Check proxy health
    if !csswitch_runtime::proxy_lifecycle::proxy_health(port, &secret) {
        return Err("代理未运行或不可达，请先运行 `csswitch proxy start`".to_string());
    }

    let env_vars = proxy_env_vars(port, &secret);
    let mut cmd = Command::new(&cmd_args[0]);
    cmd.args(&cmd_args[1..]);
    for (k, v) in &env_vars {
        cmd.env(k, v);
    }

    let status = cmd.status().map_err(|e| format!("执行失败：{e}"))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

// ── Hook commands ──

const HOOK_MARKER: &str = "# CSSWITCH_HOOK";

fn handle_hook(args: &Args) -> Result<(), String> {
    match args.subcommand.as_str() {
        "install" => {
            let shell = flag_value(&args.flags, "shell").unwrap_or_else(|| "bash".to_string());
            let rc_file = shell_rc_file(&shell)?;

            let hook_script = format!(
                "{}\ncsswitch() {{\n  command csswitch \"$@\"\n}}\n",
                HOOK_MARKER
            );

            let existing = std::fs::read_to_string(&rc_file).unwrap_or_default();
            if existing.contains(HOOK_MARKER) {
                println!("Hook 已安装在 {} 中", rc_file.display());
                return Ok(());
            }

            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&rc_file)
                .map_err(|e| format!("无法打开 {}：{e}", rc_file.display()))?;
            writeln!(f, "\n{}", hook_script)
                .map_err(|e| format!("写入失败：{e}"))?;

            println!("Hook 已安装到 {}。重新打开终端或 `source {}` 生效。", rc_file.display(), rc_file.display());
            Ok(())
        }
        "uninstall" => {
            let shell = flag_value(&args.flags, "shell").unwrap_or_else(|| "bash".to_string());
            let rc_file = shell_rc_file(&shell)?;
            let content = std::fs::read_to_string(&rc_file).unwrap_or_default();
            let new_content: String = content
                .lines()
                .filter(|line| !line.contains(HOOK_MARKER))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&rc_file, new_content.trim_end().to_string() + "\n")
                .map_err(|e| format!("写入失败：{e}"))?;
            println!("Hook 已从 {} 中移除", rc_file.display());
            Ok(())
        }
        _ => Err("用法：csswitch hook install|uninstall [--shell bash|zsh|fish]".to_string()),
    }
}

fn shell_rc_file(shell: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("找不到 HOME 目录")?;
    match shell {
        "bash" => Ok(home.join(".bashrc")),
        "zsh" => Ok(home.join(".zshrc")),
        "fish" => Ok(home.join(".config").join("fish").join("config.fish")),
        other => Err(format!("不支持的 shell：{other}（支持 bash, zsh, fish）")),
    }
}

// ── Daemon commands ──

fn proxy_env_file_path() -> PathBuf {
    std::env::temp_dir().join("csswitch-proxy.env")
}

fn write_proxy_env_file(port: u16, secret: &str) -> Result<(), String> {
    let env_vars = proxy_env_vars(port, secret);
    let content: String = env_vars
        .iter()
        .map(|(k, v)| format!("export {}=\"{}\"", k, v))
        .collect::<Vec<_>>()
        .join("\n");

    std::fs::write(proxy_env_file_path(), content)
        .map_err(|e| format!("写入代理环境文件失败：{e}"))?;
    Ok(())
}

fn handle_daemon(ctx: &dyn RuntimeContext, args: &Args) -> Result<(), String> {
    match args.subcommand.as_str() {
        "start" => {
            let state = Arc::new(Mutex::new(AppState::default()));
            let dir = csswitch_config::default_dir();
            let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
            let (secret, port, action) = ensure_proxy(ctx, &state, &cfg, 1)?;
            // Persist generated secret to config
            csswitch_config::update(&dir, |c| c.secret = secret.clone())
                .map_err(|e| e.to_string())?;
            write_proxy_env_file(port, &secret)?;

            let pid_path = daemon_pid_file();
            let pid = std::process::id();
            std::fs::write(&pid_path, pid.to_string())
                .map_err(|e| format!("写入 PID 文件失败：{e}"))?;

            let msg = match action {
                ProxyAction::Reused => "Daemon 已在运行",
                ProxyAction::Restarted => "Daemon 已启动",
            };
            println!("{msg} (PID {pid}, 端口 {port})");
            println!("代理环境文件: {}", proxy_env_file_path().display());
            println!("使用 eval \"$(csswitch env)\" 注入环境变量");
            Ok(())
        }
        "stop" => {
            let pid_path = daemon_pid_file();
            if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
                if let Ok(pid) = pid_str.trim().parse::<i32>() {
                    if pid != std::process::id() as i32 {
                        unsafe { libc::kill(pid, libc::SIGTERM); }
                    }
                }
            }
            let _ = std::fs::remove_file(&pid_path);
            let _ = std::fs::remove_file(proxy_env_file_path());
            println!("Daemon 已停止");
            Ok(())
        }
        "status" => {
            let pid_path = daemon_pid_file();
            match std::fs::read_to_string(&pid_path) {
                Ok(pid_str) => {
                    let pid: i32 = pid_str.trim().parse().unwrap_or(0);
                    let alive = unsafe { libc::kill(pid, 0) == 0 };
                    if alive {
                        let dir = csswitch_config::default_dir();
                        if let Ok(cfg) = csswitch_config::load_from(&dir) {
                            let port = cfg.proxy_port;
                            let healthy = csswitch_runtime::proxy_lifecycle::proxy_health(port, &cfg.secret);
                            if healthy {
                                println!("Daemon: 运行中 (PID {pid}, 端口 {port})");
                            } else {
                                println!("Daemon: PID {pid} 存在但代理探活失败");
                            }
                        } else {
                            println!("Daemon: PID {pid} 运行中（无法读取配置）");
                        }
                    } else {
                        println!("Daemon: 未运行（PID 文件残留: {pid}）");
                    }
                }
                Err(_) => println!("Daemon: 未运行"),
            }
            Ok(())
        }
        _ => Err("用法：csswitch daemon start|stop|status".to_string()),
    }
}

// ── Doctor ──

fn handle_doctor() -> Result<(), String> {
    println!("CSSwitch doctor（只读诊断）");
    println!();

    let dir = csswitch_config::default_dir();
    match csswitch_config::load_from(&dir) {
        Ok(cfg) => {
            println!("✓ 配置文件: {}", dir.join("config.json").display());
            println!("  Schema v{}, {} 个 profile", cfg.schema_version, cfg.profiles.len());
            if let Some(active) = cfg.active_profile() {
                let key_status = if active.api_key.is_empty() { "无key" } else { "有key" };
                println!("  生效: {} ({}) {}", active.name, active.template_id, key_status);
            } else {
                println!("  (未设置生效 profile)");
            }
        }
        Err(e) => println!("✗ 配置文件读取失败: {e}"),
    }

    println!();
    println!("[Gateway]");
    let ctx = CliContext;
    match find_gateway_bin(&ctx) {
        Some(path) => println!("✓ Gateway: {}", path.display()),
        None => println!("✗ Gateway 未找到。请先：cargo build --release -p csswitch-gateway"),
    }

    println!();
    println!("[Claude Science]");
    match find_science_bin() {
        Some(path) => println!("✓ claude-science: {}", path.display()),
        None => println!("✗ claude-science 未找到。npm install -g @anthropic-ai/claude-science"),
    }

    println!();
    println!("[端口]");
    let cfg = csswitch_config::load_from(&dir).unwrap_or_default();
    println!("  代理端口: {}  (默认: 18991)", cfg.proxy_port);
    println!("  沙箱端口: {}  (默认: 8990)", cfg.sandbox_port);

    println!();
    println!("诊断完成");
    Ok(())
}

// ── Config ──

fn handle_config() -> Result<(), String> {
    let dir = csswitch_config::default_dir();
    let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    // Redact keys
    let redacted = cfg.profiles.iter().fold(json, |s, p| {
        if p.api_key.is_empty() { s } else { s.replace(&p.api_key, &csswitch_config::mask(&p.api_key)) }
    });
    println!("{redacted}");
    Ok(())
}

// ── Env ──

fn handle_env() -> Result<(), String> {
    let dir = csswitch_config::default_dir();
    let cfg = csswitch_config::load_from(&dir).map_err(|e| e.to_string())?;
    if cfg.secret.is_empty() {
        return Err("代理尚未启动。请先运行 `csswitch proxy start` 或 `csswitch daemon start`".to_string());
    }
    let vars = proxy_env_vars(cfg.proxy_port, &cfg.secret);
    for (k, v) in &vars {
        println!("export {}=\"{}\"", k, v);
    }
    Ok(())
}
