<p align="center">
  <img src="docs/assets/social-preview.png" alt="CSSwitch Linux" width="760">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License">
  <img src="https://img.shields.io/badge/release-v1.2.0-2ea44f.svg" alt="CSSwitch Linux v1.2.0">
  <img src="https://img.shields.io/badge/platform-Linux%20|%20WSL%20|%20headless-1d1d1f.svg" alt="Linux">
  <img src="https://img.shields.io/badge/built%20with-Rust%20|%20Python%20|%20Tauri%202-dca282.svg" alt="Rust">
</p>

<p align="center">
  <a href="./README.md">简体中文</a> ·
  <a href="./README.en.md">English</a>
</p>

# CSSwitch Linux

CSSwitch Linux 是基于 [CSSwitch v0.4.4](https://github.com/SuperJJ007/CSSwitch) 移植的 **Linux 原生实现**，支持 CLI 命令行和 Tauri 桌面 GUI 两种使用方式。它把 Claude Science 的推理请求转换并接入你自己的模型 API，支持 DeepSeek、通义千问、Kimi、智谱 GLM、小米 MiMo、硅基流动、MiniMax、OpenRouter 或自定义兼容端点。

> 本项目基于 CSSwitch macOS v0.4.4（[SuperJJ007/CSSwitch](https://github.com/SuperJJ007/CSSwitch)）移植。感谢原作者的杰出工作。

## 与 macOS 桌面版的区别

|                              | macOS 桌面版 (v0.4.4)   | Linux v1.2.0                    |
| ---------------------------- | ------------------------ | ------------------------------- |
| **界面**                     | Tauri 2 菜单栏面板       | CLI + 桌面 GUI（Tauri 2 系统托盘）|
| **平台**                     | macOS Apple Silicon      | Linux / WSL / headless           |
| **Science 安装**             | `.app` 应用包            | `npm install -g`                |
| **安装方式**                 | `.dmg` 拖入 Applications | `dpkg -i` / `cargo install`     |
| **代理注入**                 | 一键启动自动注入         | eval / hook / wrapper / 桌面一键 |
| **后台运行**                 | 菜单栏常驻               | `csswitch daemon start` / 桌面托盘|
| **多 profile 管理**          | 固定槽位                 | 无限 profile + 切换事务校验      |

## 系统架构

```
┌──────────────────────────────────────────────────────────┐
│                      用户界面层                           │
│  ┌──────────┐  ┌──────────────────────────────────────┐  │
│  │ CLI 命令  │  │ 桌面 GUI（Tauri 2）                     │  │
│  │ profile  │  │  系统托盘 + 配置面板 + 一键启动          │  │
│  │ proxy    │  │  └─ 窗口关闭 → 最小化到托盘               │  │
│  │ daemon   │  └──────────────────────────────────────┘  │
│  │ science  │                                            │
│  │ run/hook │                                            │
│  └────┬─────┘                                            │
└───────┼──────────────────────────────────────────────────┘
        │
┌───────┴──────────────────────────────────────────────────┐
│                    核心运行时层                            │
│  ┌─────────────────┐  ┌───────────────────────────────┐  │
│  │ csswitch-runtime │  │ desktop_lib (Tauri backend)   │  │
│  │  · 代理生命周期   │  │  · 切换事务（scratch→real→commit）│
│  │  · 沙箱管理      │  │  · 生命周期串行器 + generation    │  │
│  │  · daemon PID    │  │  · 上游探活 + 模型发现            │  │
│  └────────┬────────┘  │  · 虚拟 OAuth 登录伪造（Rust 原生）│
│           │           └───────────────┬───────────────┘  │
│           │                           │                   │
│  ┌────────┴───────────────────────────┴───────────────┐  │
│  │              csswitch-config（配置读写）              │  │
│  │  ~/.csswitch/config.json  · v2 schema  · 0600 权限  │  │
│  │  滚动备份 · v1→v2 迁移 · key 掩码 · 并发安全          │  │
│  └────────────────────────────────────────────────────┘  │
└───────────────────────┬──────────────────────────────────┘
                        │
┌───────────────────────┴──────────────────────────────────┐
│                    翻译代理层                              │
│  ┌──────────────────────────────┐  ┌──────────────────┐  │
│  │ Python 代理（proxy/）         │  │ Rust 网关（gateway/）│
│  │  · deepseek adapter: 原生透传 │  │  · Anthropic 兼容  │  │
│  │  · qwen adapter: 协议转换     │  │  · OpenAI Chat     │  │
│  │  · relay adapter: 通用中转    │  │  · DSML shim       │  │
│  │  · DSML tool-use 兜底 shim    │  │  · 鉴权剥离+替换   │  │
│  └──────────────────────────────┘  └──────────────────┘  │
│  只监听 127.0.0.1  ·  key 经环境变量注入  ·  不留日志     │
└───────────────────────┬──────────────────────────────────┘
                        │ ANTHROPIC_BASE_URL
                        ▼
┌──────────────────────────────────────────────────────────┐
│              Claude Science 沙箱                          │
│  独立 HOME: ~/.csswitch/sandbox/home                      │
│  隔离端口 · 虚拟 OAuth 登录态（AES-GCM）· 不碰真实凭证     │
└──────────────────────────────────────────────────────────┘
```

### 核心设计

**1. 翻译代理**是系统的核心，负责在 Claude Science 和第三方 API 之间翻译请求/响应。三种 adapter：

| Adapter | 说明 | 关键差异 |
|---------|------|---------|
| `deepseek` | DeepSeek 原生 Anthropic 端点 | 纯透传，不翻译协议；改鉴权头 + 模型名映射 + 夹取 max_tokens |
| `qwen` | 通义千问 DashScope | Anthropic ↔ OpenAI Chat 双向翻译，流式 SSE 回放保真 tool_use |
| `relay` | 通用中转站 | 透传到任意 Anthropic 兼容端点；支持 base_url + model override；thinking 策略注入 |

**2. 模板注册表**（`templates.rs`）是 provider 配置的单一来源。每个模板定义了 adapter、默认 base_url、内置模型列表、thinking 策略等，前端从后端拉取一次铺 UI，不复制常量。

**3. 切换事务**（`set_active_profile_txn`）保证切换 provider 的安全性：
- **scratch 校验**：候选配置在临时端口上发 Message/Models 探测上游，验证 key 有效
- **起正式代理**：通过后启动正式代理并探活
- **提交**：探活健康才落盘 active_id；失败则回滚到旧代理，**绝不停沙箱**（path-secret 持久不变）

**4. 生命周期串行器**（`lifecycle::Lifecycle`）三层锁严格避免自死锁：
- 命令级串行锁（最外层）→ 运行态 AppState 锁（内层）→ config 锁（最内层）
- 探活在 AppState 锁外执行，用 generation token 防「被清除后写回」竞态

**5. 虚拟 OAuth 登录**（`oauth_forge.rs`）Rust 原生实现 AES-GCM 加密，零 node 依赖。为沙箱创建隔离的登录态，与用户真实 Claude 账号完全隔离。

## 快速开始

### 前置条件

- Linux x86-64（Ubuntu 20.04+、Debian 11+、RHEL 8+、Arch 等）或 WSL2
- Claude Science CLI（`npm install -g @anthropic-ai/claude-science`）
- 一个可用的第三方模型 API Key

### 安装

**方式一：deb 包（Debian / Ubuntu）**

```bash
# CLI 工具（命令行必备）
curl -LO https://github.com/YuntaoOvO/CSSwitch-Linux/releases/latest/download/csswitch_1.2.0_amd64.deb
sudo dpkg -i csswitch_1.2.0_amd64.deb

# 桌面 GUI（可选，提供系统托盘和配置面板）
curl -LO https://github.com/YuntaoOvO/CSSwitch-Linux/releases/latest/download/csswitch-desktop_1.2.0_amd64.deb
sudo dpkg -i csswitch-desktop_1.2.0_amd64.deb
```

`csswitch-desktop` 依赖 `csswitch`，须先安装 CLI 包。桌面包会注册 `.desktop` 文件，安装后可从应用菜单启动。

**方式二：从源码编译**

```bash
git clone https://github.com/YuntaoOvO/CSSwitch-Linux.git
cd CSSwitch-Linux

# CLI 工具
cargo build --release -p csswitch -p csswitch-gateway
sudo cp target/release/csswitch target/release/csswitch-gateway /usr/local/bin/

# 桌面 GUI（可选）
cd desktop/src-tauri && cargo build --release
sudo cp target/release/desktop /usr/local/bin/csswitch-desktop
```

### 基本使用

```bash
# 1. 添加配置
csswitch profile add --template deepseek --name "我的DeepSeek" --key sk-xxxxxxxx

# 2. 激活（设为当前生效）
csswitch profile activate <上一步生成的ID>

# 3. 启动后台代理
csswitch daemon start

# 4. 注入环境并运行（三选一）

# A. 手动注入
eval "$(csswitch env)"
claude-science

# B. wrapper 运行
csswitch run -- claude-science "帮我分析代码库"

# C. 安装 shell hook（推荐，一次性设置）
csswitch hook install --shell bash
source ~/.bashrc
# 此后直接运行 claude-science 即可自动代理

# 5. 一键启动 Science 沙箱（代理 + 登录 + 沙箱 + 打开浏览器）
csswitch science start
```

## 命令参考

| 命令                                                 | 说明                          |
| ---------------------------------------------------- | ----------------------------- |
| `csswitch profile list`                              | 列出所有配置                  |
| `csswitch profile add --template --name --key`       | 添加新配置                    |
| `csswitch profile delete <id>`                       | 删除配置                      |
| `csswitch profile activate <id>`                     | 设为当前生效配置              |
| `csswitch profile show [id]`                         | 查看配置详情（key 掩码）      |
| `csswitch proxy start / stop / status`               | 代理控制                      |
| `csswitch daemon start / stop / status`              | 后台 daemon（PID 文件）       |
| `csswitch science start / stop / status`             | Science 沙箱一键管理          |
| `csswitch run -- <cmd> [args]`                       | 注入代理环境执行命令          |
| `csswitch hook install / uninstall --shell <sh>`     | Shell hook 管理               |
| `csswitch env`                                       | 打印代理环境变量（供 eval）   |
| `csswitch doctor`                                    | 只读环境诊断                  |
| `csswitch config`                                    | 显示完整配置（key 掩码）      |

### profile add 完整参数

```bash
csswitch profile add \
  --template deepseek|qwen|glm|kimi|siliconflow|xiaomi|openrouter|minimax|custom \
  --name "显示名称" \
  --key sk-xxxxxxxx \
  [--base-url https://...] \   # relay / custom 可选
  [--model model-name]         # relay / custom 可选
```

## 支持的模型来源

| 来源           | 模板 ID        | API 格式   | Adapter   | 说明                              |
| -------------- | -------------- | ---------- | --------- | --------------------------------- |
| DeepSeek       | `deepseek`     | anthropic  | deepseek  | 原生 Anthropic 端点，支持 DSML shim |
| 通义千问       | `qwen`         | openai_chat| qwen      | 代理做 Anthropic ↔ OpenAI 协议转换 |
| 智谱 GLM       | `glm`          | anthropic  | relay     | Anthropic 兼容端点透传 + model force |
| 小米 MiMo      | `xiaomi`       | anthropic  | relay     | Anthropic 兼容端点透传 + model force |
| 硅基流动       | `siliconflow`  | anthropic  | relay     | relay + 多模型 force              |
| Kimi（Moonshot）| `kimi`        | anthropic  | relay     | Anthropic 兼容，强制 thinking=enabled |
| MiniMax        | `minimax`      | anthropic  | relay     | Anthropic 兼容端点透传 + model force |
| OpenRouter     | `openrouter`   | anthropic  | relay     | 国际中转站 + model force          |
| 自定义         | `custom`       | anthropic  | relay     | 任意 Anthropic 兼容端点           |

**Adapter 行为说明：**
- **deepseek / qwen**：走各自官方硬编码端点，base_url 只读、不可编辑
- **relay 家族**（glm / xiaomi / siliconflow / kimi / minimax / openrouter / custom）：base_url 可编辑，支持 model override 和 thinking 策略注入。空 model 会退回 passthrough 模式（Science 显示 claude），故 relay 家族强制要求填写 model

## 如何保护你的真实账号

- 不复制、读取或修改真实 Claude 登录凭证
- 隔离 Science 使用独立 HOME（`~/.csswitch/sandbox/home`）和独立端口
- 第三方 API Key 保存在 `~/.csswitch/config.json`，文件权限 `0600`
- Key 只经环境变量注入代理子进程，绝不出现于命令行参数（防 ps 泄露）
- 日志中的 secret 和 key 已脱敏回显；本地代理只监听 `127.0.0.1`
- 虚拟 OAuth 登录态与真实凭证完全隔离，用 Rust 原生 AES-GCM 加密

## 开发

```bash
git clone https://github.com/YuntaoOvO/CSSwitch-Linux.git
cd CSSwitch-Linux

# 编译 CLI
cargo build --release -p csswitch -p csswitch-gateway

# 编译桌面 GUI
cd desktop/src-tauri && cargo build --release && cd ../..

# 测试
cargo test --workspace

# 打包
bash scripts/build-deb.sh               # CLI deb → releases/csswitch_1.2.0_amd64.deb
bash scripts/build-deb-desktop.sh       # 桌面 deb → releases/csswitch-desktop_1.2.0_amd64.deb
cargo install --path cli               # 安装 csswitch 到 ~/.cargo/bin
```

### Workspace 结构

```
├── cli/                          # csswitch CLI（手写 arg parser，~750 行）
│   └── main.rs                   #   命令分发 + profile/proxy/daemon/science/run/hook/env/doctor/config
├── crates/
│   ├── csswitch-config/          # 配置读写（~/.csswitch/config.json，v2 schema，读写锁+滚动备份）
│   ├── csswitch-templates/       # 模板注册表（12 个 provider，单一来源）
│   ├── csswitch-oauth/           # 虚拟 OAuth 登录（AES-GCM 纯 Rust，零 node 依赖）
│   └── csswitch-runtime/         # 核心运行时（proxy lifecycle / sandbox / provider / system）
├── desktop/
│   ├── src/                      # 桌面前端（Tauri webview UI）
│   ├── src-tauri/
│   │   ├── icons/                # 应用图标（32×32 → 256×256 + tray 状态图标）
│   │   ├── src/
│   │   │   ├── lib.rs            #   桌面后端核心（~2500 行）：全部 Tauri commands + 代理/沙箱生命周期
│   │   │   ├── main.rs           #   入口
│   │   │   ├── tray.rs           #   系统托盘（绿色=健康 / 黄色=异常 / 灰色=未运行）
│   │   │   ├── lifecycle.rs      #   生命周期串行器 + generation token
│   │   │   ├── templates.rs      #   模板注册表（9 个 provider 模板）
│   │   │   ├── proc.rs           #   进程管理 + HTTP 探活 + 上游可达性探测
│   │   │   ├── scratch.rs        #   上游 scratch 校验（临时代理探测，不碰正式链路）
│   │   │   ├── config.rs         #   配置读写（复用 csswitch-config 范式，桌面独立复本）
│   │   │   ├── config_legacy.rs  #   v1→v2 配置迁移
│   │   │   └── oauth_forge.rs    #   虚拟 OAuth 登录伪造（纯 Rust AES-GCM）
│   │   ├── tauri.conf.json       #   Tauri 2 配置（窗口/打包/安全策略）
│   │   └── Cargo.toml            #   桌面端依赖
│   └── gateway/                  # csswitch-gateway（Rust 翻译代理 sidecar）
│       └── src/
│           ├── main.rs           #   入口 + HTTP server
│           ├── server.rs         #   请求路由（path-secret 前缀 + CORS）
│           ├── messages.rs       #   /v1/messages 处理（anthropic/openai_chat/openai_responses）
│           ├── models.rs         #   /v1/models 动态发现
│           ├── anthropic_compat.rs#  Anthropic 兼容层
│           ├── openai_chat.rs    #   OpenAI Chat Completions 翻译
│           ├── openai_responses.rs#  OpenAI Responses 翻译
│           ├── auth.rs           #   鉴权剥离 + 替换
│           ├── connect.rs        #   上游连接（HTTPS + 重试 + 超时）
│           ├── dsml_shim.rs      #   DSML tool-use 兜底 shim
│           ├── policy.rs         #   速率限制 + 安全策略
│           └── config.rs         #   网关配置
├── proxy/
│   ├── csswitch_proxy.py         # Python 代理（deepseek/qwen/relay adapter）
│   └── dsml_shim.py              # DSML tool-use 兜底 shim（Python 版）
├── scripts/
│   ├── build-deb.sh              # CLI deb 打包
│   ├── build-deb-desktop.sh      # 桌面 deb 打包（含图标 + .desktop + postinst）
│   ├── launch-virtual-sandbox.sh # 启动虚拟 Science 沙箱
│   ├── stop-science-sandbox.sh   # 停止沙箱
│   ├── doctor.sh                 # 环境诊断脚本
│   └── ...                       # 其他维护脚本
├── csswitch.desktop              # Linux 桌面入口文件（Icon=csswitch）
└── releases/                     # 构建产物（*.deb）
```

### 数据流：一键启动 Science

```
用户点击"一键开始"
  │
  ▼
1. ensure_proxy()
   ├─ 读生效 profile（active_id → config.json）
   ├─ 模板 → adapter（deepseek/qwen/relay）
   ├─ 检查复用：端口 + adapter + key 指纹一致且健康 → 跳过
   ├─ 起代理子进程：python3 csswitch_proxy.py --provider <adapter> --port <port> --auth-token <secret>
   │  └─ key 经环境变量注入（DEEPSEEK_API_KEY / DASHSCOPE_API_KEY / CSSWITCH_RELAY_KEY）
   └─ 探活：轮询 /health 最多 4s
  │
  ▼
2. 虚拟 OAuth 登录（幂等）
   ├─ 检查 ~/.csswitch/sandbox/home/.claude-science/ 登录态
   └─ 缺失/过期 → AES-GCM 锻造新凭证
  │
  ▼
3. 启动 Science 沙箱
   ├─ bash scripts/launch-virtual-sandbox.sh --port 8990 --proxy-url http://127.0.0.1:18991/<secret>
   │  └─ ANTHROPIC_BASE_URL=http://127.0.0.1:18991/<secret>
   │  └─ HOME=~/.csswitch/sandbox/home（隔离）
   └─ 轮询 sandbox /health 最多 8s
  │
  ▼
4. 打开浏览器
   └─ claude-science url --data-dir <sandbox_home>/.claude-science
```

### 关键安全设计

- **Key 绝不进 argv**：所有 API key 经 `cmd.env()` 注入，`ps aux` 不可见
- **Config 0600**：`~/.csswitch/config.json` 设为仅 owner 可读写
- **日志脱敏**：proxy secret 出现在日志中时替换为 `****` 后才回显
- **滚动备份净化**：清 key / 删 profile 时同步删除 `.bak` 备份，旧明文不可恢复
- **O_NOFOLLOW**：日志文件打开时带 `O_NOFOLLOW` 标志，防符号链接攻击
- **只监听回环**：代理和网关只 bind `127.0.0.1`，不接受外部连接
- **path-secret 前缀**：网关 URL 含随机 secret 前缀，只有持有 secret 的进程能连通
- **并发安全**：generation token 防「清 key 后旧 key 写入运行态」；串行器防命令交叠

## 致谢

- [CSSwitch](https://github.com/SuperJJ007/CSSwitch)（SuperJJ007）— 本项目基于其 v0.4.4 macOS 桌面版移植
- [CC Switch](https://github.com/farion1231/cc-switch) — 产品形态参考

## 许可

[MIT](./LICENSE)
