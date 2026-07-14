# CSSwitch Linux

A pure CLI port of [CSSwitch v0.4.4](https://github.com/SuperJJ007/CSSwitch) for **Linux, WSL, and headless environments**. Route Claude Science inference requests through your own model API (DeepSeek, Qwen, Kimi, GLM, OpenRouter, custom endpoints).

Built with Rust. No GUI, no Tauri, no Node.js runtime required.

## Quick Start

```bash
npm install -g @anthropic-ai/claude-science
git clone https://github.com/YuntaoOvO/CSSwitch-Linux.git
cd CSSwitch-Linux
cargo build --release -p csswitch-gateway -p csswitch
sudo cp target/release/csswitch target/release/csswitch-gateway /usr/local/bin/

csswitch profile add --template deepseek --name "DS" --key sk-xxx
csswitch profile activate <id>
csswitch daemon start
eval "$(csswitch env)"
claude-science
```

Full documentation: [简体中文 README](./README.md)

This project is based on [CSSwitch](https://github.com/SuperJJ007/CSSwitch) by SuperJJ007.

License: [MIT](./LICENSE)
