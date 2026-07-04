#!/usr/bin/env bash
# 停止隔离沙箱 Science（只停沙箱 data-dir 的守护进程，绝不影响真实实例 8765）。
# 平台：支持 macOS (Darwin) 与 Linux。
set -euo pipefail
PROJ="$(cd "$(dirname "$0")/.." && pwd)"
SANDBOX_HOME="${SANDBOX_HOME:-$PROJ/.sandbox/home}"
DATA_DIR="$SANDBOX_HOME/.claude-science"

# —— 平台检测：默认 Science 二进制路径 ——
OS="$(uname -s)"
if [[ "$OS" == "Darwin" ]]; then
  DEFAULT_BIN="/Applications/Claude Science.app/Contents/Resources/bin/claude-science"
else
  DEFAULT_BIN="$HOME/.local/bin/claude-science"
fi
BIN="${SCIENCE_BIN:-$DEFAULT_BIN}"

if [[ ! -d "$DATA_DIR" ]]; then echo "沙箱不存在，无需停止。"; exit 0; fi

if HOME="$SANDBOX_HOME" "$BIN" stop --data-dir "$DATA_DIR" 2>&1 | tail -2; then
  echo "沙箱已停。真实实例 8765 未受影响。"
else
  rc=${PIPESTATUS[0]:-$?}
  echo "停止失败（退出码 $rc）。真实实例 8765 未受影响。" >&2
  exit "$rc"
fi
