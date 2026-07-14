<p align="center">
  <img src="docs/assets/social-preview.png" alt="CSSwitch Linux" width="760">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License">
  <img src="https://img.shields.io/badge/release-v0.5.0-2ea44f.svg" alt="CSSwitch Linux v0.5.0">
  <img src="https://img.shields.io/badge/platform-Linux%20|%20WSL%20|%20headless-1d1d1f.svg" alt="Linux">
  <img src="https://img.shields.io/badge/built%20with-Rust-dca282.svg" alt="Rust">
</p>

<p align="center">
  <a href="./README.md">简体中文</a>
</p>

# CSSwitch Linux

CSSwitch Linux 是基于 [CSSwitch v0.4.4](https://github.com/SuperJJ007/CSSwitch) 移植的纯 CLI 版本，专为 **Linux、WSL 和无 GUI 设备**打造。它把 Claude Science 的推理请求转换并接入你自己的模型 API，支持 DeepSeek、通义千问、Kimi、GLM、硅基流动、OpenRouter 或自定义兼容端点。

核心能力与 macOS 桌面版一致：管理多套 provider 配置、启动本地 Rust 翻译网关、自动准备隔离 OAuth 登录环境、注入代理环境变量运行 Claude Science。

> 本项目基于 CSSwitch macOS v0.4.4（[SuperJJ007/CSSwitch](https://github.com/SuperJJ007/CSSwitch)）移植，所有协议转换、OAuth 伪造、配置管理等核心逻辑与原版对齐。感谢原作者的杰出工作。

## 与 macOS 桌面版的区别

| | macOS 桌面版 (v0.4.4) | Linux CLI 版 (v0.5.0) |
|---|---|---|
| **界面** | Tauri 2 菜单栏面板 | 纯 CLI（手写参数解析） |
| **平台** | macOS Apple Silicon | Linux / WSL / headless |
| **Science 安装** | `.app` 应用包 | `npm install -g @anthropic-ai/claude-science` |
| **代理注入** | 自动，一键启动 | `csswitch run --`、`eval` 注入、shell hook |
| **后台运行** | 菜单栏常驻 | `csswitch daemon start` + PID 文件 |

## 快速开始

### 前置条件

- Linux（Ubuntu 20.04+、Debian、Arch 等）或 WSL2
- Rust 工具链（`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`）
- Node.js 环境（用于安装 Claude Science）
- 一个可用的第三方模型 API Key

### 安装

```bash
# 1. 安装 Claude Science CLI
npm install -g @anthropic-ai/claude-science

# 2. 克隆并编译 CSSwitch Linux
git clone https://github.com/YuntaoOvO/CSSwitch-Linux.git
cd CSSwitch-Linux
cargo build --release -p csswitch-gateway
cargo build --release -p csswitch

# 3. 将编译产物放到 PATH
sudo cp target/release/csswitch /usr/local/bin/
sudo cp target/release/csswitch-gateway /usr/local/bin/
```

### 基本使用

```bash
# 添加一个 DeepSeek 配置
csswitch profile add --template deepseek --name "我的DeepSeek" --key sk-xxxxxxxx

# 激活该配置
csswitch profile activate <上一步生成的ID>

# 启动代理 daemon（后台常驻）
csswitch daemon start

# 方式 A：注入环境后运行
eval "$(csswitch env)"
claude-science

# 方式 B：wrapper 运行（自动注入）
csswitch run -- claude-science "帮我分析当前代码库"

# 方式 C：安装 shell hook（推荐）
csswitch hook install --shell bash
# 重新打开终端后，直接运行 claude-science 即可自动代理

# 查看状态
csswitch proxy status
csswitch daemon status

# 环境诊断
csswitch doctor
```

## CLI 命令参考

| 命令 | 说明 |
|---|---|
| `csswitch profile list` | 列出所有配置 |
| `csswitch profile add --template --name --key` | 添加新配置 |
| `csswitch profile delete <id>` | 删除配置 |
| `csswitch profile activate <id>` | 设为当前生效配置 |
| `csswitch profile show [id]` | 查看配置详情（key 掩码） |
| `csswitch proxy start/stop/status` | 代理控制 |
| `csswitch daemon start/stop/status` | 后台 daemon |
| `csswitch science start/stop/status` | Science 沙箱一键管理 |
| `csswitch run -- <cmd> [args]` | 注入代理环境执行 |
| `csswitch hook install/uninstall --shell` | Shell hook 管理 |
| `csswitch env` | 打印代理环境变量 |
| `csswitch doctor` | 只读环境诊断 |
| `csswitch config` | 显示完整配置（key 掩码） |

## 支持的模型来源

| 来源 | 类型 | 说明 |
|---|---|---|
| DeepSeek | 国内官方 | 原生 Anthropic 兼容，支持 tool-use shim |
| 通义千问 | 国内官方 | 通过代理做 Anthropic ↔ OpenAI 协议转换 |
| 智谱 GLM | 国内官方 | 原生 Anthropic 兼容端点透传 |
| Kimi（Moonshot） | 国内官方 | 原生 Anthropic 兼容端点透传 |
| 小米 MiMo | 国内官方 | 原生 Anthropic 兼容端点透传 |
| 硅基流动 | 国内中转 | relay + model override |
| OpenRouter | 国际中转 | relay + model override |
| 自定义 Anthropic | 自填 | 适合私有网关、兼容中转站 |
| 自定义 OpenAI | 自填 | OpenAI Chat Completions 兼容 |
| 自定义 OpenAI Responses | 自填 | OpenAI Responses 兼容 |

## 如何保护你的真实账号

- 不复制、读取或修改真实 Claude 登录凭证
- 隔离 Science 使用独立 HOME（`~/.csswitch/sandbox/home`）和独立端口
- 第三方 API Key 保存在 `~/.csswitch/config.json`，文件权限 `0600`
- Key 不显示在日志中，本地网关只监听 `127.0.0.1`

## 开发

```bash
# 编译检查
cargo check --workspace

# 运行测试
cargo test --workspace

# 仅构建 CLI
cargo build --release -p csswitch
cargo build --release -p csswitch-gateway
```

## 架构

```
CSSwitch profile
  → csswitch-gateway (Rust 本地网关，Anthropic ↔ 各 provider 协议转换)
  → 隔离 OAuth 登录态
  → Claude Science (npm 全局安装)
  → 注入 ANTHROPIC_BASE_URL + 独立 HOME
```

### Workspace 结构

```
CSSwitch-Linux/
├── cli/                      # csswitch CLI（手写 arg parser，零外部解析依赖）
├── crates/
│   ├── csswitch-config/      # 配置读写（~/.csswitch/config.json, v2 schema）
│   ├── csswitch-templates/   # 模板注册表（12 个 provider）
│   ├── csswitch-oauth/       # 虚拟 OAuth 登录（AES-GCM）
│   └── csswitch-runtime/     # 核心运行时（proxy/sandbox/provider/system）
├── desktop/gateway/          # csswitch-gateway（翻译代理 sidecar）
└── desktop/src-tauri/        # macOS 桌面版（保持原样，不在 workspace 中）
```

## 致谢

- [CSSwitch](https://github.com/SuperJJ007/CSSwitch)（SuperJJ007）— 本项目基于其 v0.4.4 macOS 桌面版移植
- [CC Switch](https://github.com/farion1231/cc-switch) — 产品形态参考

## 许可

[MIT](./LICENSE)
