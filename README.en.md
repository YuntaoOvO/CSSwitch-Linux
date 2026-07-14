# CSSwitch Linux

A pure CLI port of [CSSwitch v0.4.4](https://github.com/SuperJJ007/CSSwitch) for **Linux, WSL, and headless environments**. Route Claude Science inference through your own model API.

## Install

**Via deb (Debian/Ubuntu):**

```bash
curl -LO https://github.com/YuntaoOvO/CSSwitch-Linux/releases/latest/download/csswitch_1.1.1_amd64.deb
sudo dpkg -i csswitch_1.1.1_amd64.deb
```

**From source:**

```bash
git clone https://github.com/YuntaoOvO/CSSwitch-Linux.git
cd CSSwitch-Linux
make && sudo make install
```

## Usage

```bash
csswitch profile add --template deepseek --name "DS" --key sk-xxx
csswitch profile activate <id>
csswitch daemon start
eval "$(csswitch env)"
claude-science
```

Full documentation: [简体中文 README](./README.md)

## Build

```bash
cargo build --release -p csswitch -p csswitch-gateway
make deb          # → releases/csswitch_1.1.0_amd64.deb
make install      # → /usr/local/bin/csswitch
```

This project is based on [CSSwitch](https://github.com/SuperJJ007/CSSwitch) by SuperJJ007.

License: [MIT](./LICENSE)
