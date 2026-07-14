# CSSwitch Linux

A pure CLI + desktop GUI port of [CSSwitch v0.4.4](https://github.com/SuperJJ007/CSSwitch) for **Linux, WSL, and headless environments**. Route Claude Science inference through your own model API — supports DeepSeek, Qwen, GLM, Kimi, MiMo, SiliconFlow, MiniMax, OpenRouter, or any custom Anthropic-compatible endpoint.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                  User Interface                  │
│  ┌──────────┐  ┌──────────────────────────┐     │
│  │ CLI       │  │ Desktop GUI (Tauri 2)    │     │
│  │ commands  │  │ System tray + panel +    │     │
│  │           │  │ one-click launcher       │     │
│  └─────┬─────┘  └───────────┬──────────────┘     │
└────────┼────────────────────┼────────────────────┘
         │                    │
    ┌────┴────────────────────┴─────┐
    │       Core Runtime              │
    │  · Profile management           │
    │  · Switch transaction           │
    │  · Proxy/sandbox lifecycle      │
    │  · Virtual OAuth (AES-GCM)      │
    └─────────────┬───────────────────┘
                  │
    ┌─────────────┴───────────────────┐
    │      Translation Proxy          │
    │  · DeepSeek: native passthrough │
    │  · Qwen: protocol translation   │
    │  · Relay: generic forwarder     │
    │  localhost-only · key via env   │
    └─────────────┬───────────────────┘
                  │ ANTHROPIC_BASE_URL
                  ▼
    ┌─────────────────────────────────┐
    │   Claude Science Sandbox        │
    │   Isolated HOME · separate port │
    │   Virtual credentials only      │
    └─────────────────────────────────┘
```

### How It Works

1. **Translation Proxy** — A local proxy translates Claude Science's Anthropic API calls to your chosen provider's format. Three adapter modes: `deepseek` (native Anthropic passthrough), `qwen` (Anthropic↔OpenAI translation), and `relay` (transparent forwarding to any Anthropic-compatible endpoint).

2. **Template Registry** — Single source of truth for all 9 provider templates (DeepSeek, GLM, Xiaomi, SiliconFlow, Kimi, MiniMax, OpenRouter, Qwen, Custom). Each template defines the adapter, default base_url, built-in model list, and thinking policy.

3. **Switch Transaction** — Activating a profile runs a three-phase safety check: (a) scratch-validate the candidate config against the upstream API on a temporary port, (b) start the real proxy and health-check it, (c) only commit the active_id to disk if healthy — otherwise roll back to the previous config.

4. **Virtual OAuth Login** — Pure Rust AES-GCM forged credentials give Claude Science a valid login session without touching the user's real account. Fully isolated to `~/.csswitch/sandbox/home`.

5. **Security by Design** — API keys injected via environment variables (never in command-line arguments), config file permission `0600`, secret redaction in logs, `O_NOFOLLOW` on log files, localhost-only listening, path-secret prefix on gateway URLs.

## Install

**Via deb (Debian/Ubuntu):**

```bash
# CLI tools (required)
curl -LO https://github.com/YuntaoOvO/CSSwitch-Linux/releases/latest/download/csswitch_1.2.0_amd64.deb
sudo dpkg -i csswitch_1.2.0_amd64.deb

# Desktop GUI (optional — system tray + config panel)
curl -LO https://github.com/YuntaoOvO/CSSwitch-Linux/releases/latest/download/csswitch-desktop_1.2.0_amd64.deb
sudo dpkg -i csswitch-desktop_1.2.0_amd64.deb
```

**From source:**

```bash
git clone https://github.com/YuntaoOvO/CSSwitch-Linux.git
cd CSSwitch-Linux

# CLI
cargo build --release -p csswitch -p csswitch-gateway
sudo cp target/release/csswitch target/release/csswitch-gateway /usr/local/bin/

# Desktop GUI (optional)
cd desktop/src-tauri && cargo build --release
sudo cp target/release/desktop /usr/local/bin/csswitch-desktop
```

## Usage

```bash
# 1. Add a profile
csswitch profile add --template deepseek --name "DS" --key sk-xxx

# 2. Activate it
csswitch profile activate <id>

# 3. Start the daemon (background proxy)
csswitch daemon start

# 4. Inject proxy environment and run
eval "$(csswitch env)"
claude-science

# Or use the wrapper
csswitch run -- claude-science "analyze my codebase"

# Or install shell hook (one-time setup)
csswitch hook install --shell bash
source ~/.bashrc

# 5. One-click sandbox launch
csswitch science start

# Diagnostics
csswitch doctor
```

## Command Reference

| Command                                              | Description                       |
| ---------------------------------------------------- | --------------------------------- |
| `csswitch profile list`                              | List all profiles                 |
| `csswitch profile add --template --name --key`       | Add a new profile                 |
| `csswitch profile delete <id>`                       | Delete a profile                  |
| `csswitch profile activate <id>`                     | Set as active profile             |
| `csswitch profile show [id]`                         | Show profile details (key masked) |
| `csswitch proxy start / stop / status`               | Proxy lifecycle                   |
| `csswitch daemon start / stop / status`              | Daemon management                 |
| `csswitch science start / stop / status`             | Science sandbox management         |
| `csswitch run -- <cmd> [args]`                       | Run command with proxy env        |
| `csswitch hook install / uninstall --shell <sh>`     | Shell hook management             |
| `csswitch env`                                       | Print proxy env vars (for eval)   |
| `csswitch doctor`                                    | Read-only diagnostics             |
| `csswitch config`                                    | Show config (keys redacted)       |

## Supported Providers

| Provider       | Template ID    | API Format  | Adapter   | Notes                                 |
| -------------- | -------------- | ----------- | --------- | ------------------------------------- |
| DeepSeek       | `deepseek`     | anthropic   | deepseek  | Native Anthropic endpoint + DSML shim |
| Qwen (Alibaba) | `qwen`         | openai_chat | qwen      | Anthropic↔OpenAI protocol translation |
| GLM (Zhipu)    | `glm`          | anthropic   | relay     | Compatible endpoint + model force     |
| MiMo (Xiaomi)  | `xiaomi`       | anthropic   | relay     | Compatible endpoint + model force     |
| SiliconFlow    | `siliconflow`  | anthropic   | relay     | Relay + multi-model force             |
| Kimi (Moonshot)| `kimi`         | anthropic   | relay     | Compatible, thinking=enabled forced   |
| MiniMax        | `minimax`      | anthropic   | relay     | Compatible endpoint + model force     |
| OpenRouter     | `openrouter`   | anthropic   | relay     | International relay + model force     |
| Custom         | `custom`       | anthropic   | relay     | Any Anthropic-compatible endpoint     |

## Build

```bash
# Build binaries
cargo build --release -p csswitch -p csswitch-gateway

# Desktop
cd desktop/src-tauri && cargo build --release && cd ../..

# Run tests
cargo test --workspace

# Package debs
bash scripts/build-deb.sh               # CLI → releases/csswitch_1.2.0_amd64.deb
bash scripts/build-deb-desktop.sh       # Desktop → releases/csswitch-desktop_1.2.0_amd64.deb

# Install locally
cargo install --path cli
```

## Project Layout

```
cli/                  CLI entry point
crates/
  csswitch-config/    Config read/write (v2 schema, 0600, rolling backup)
  csswitch-templates/ Provider template registry
  csswitch-oauth/     Virtual OAuth login (AES-GCM, pure Rust)
  csswitch-runtime/   Core runtime (proxy/sandbox/provider/system)
desktop/
  src/                Frontend (Tauri webview UI)
  src-tauri/          Backend (Tauri 2, ~2500 loc Rust)
    icons/            App + tray status icons
    src/lib.rs        All Tauri commands + lifecycle
    src/tray.rs       System tray (green/amber/gray status)
    src/templates.rs  Template registry (9 providers)
  gateway/            Rust translation proxy sidecar
proxy/                Python proxy (deepseek/qwen/relay adapters)
scripts/              Build/packaging/maintenance scripts
```

## Security

- API keys injected via environment variables — never in command-line arguments
- `~/.csswitch/config.json` permission `0600`
- All proxy/gateway listeners bind to `127.0.0.1` only
- Secret tokens redacted in log output before display
- Rolling config backup purged when keys are cleared or profiles deleted
- `O_NOFOLLOW` on log files prevents symlink attacks
- Virtual OAuth credentials are cryptographically isolated from real account data

Full documentation: [简体中文 README](./README.md)

This project is based on [CSSwitch](https://github.com/SuperJJ007/CSSwitch) by SuperJJ007.

License: [MIT](./LICENSE)
