#!/usr/bin/env bash
# CSSwitch 真机验收护栏。
#
# 只管理独立测试 HOME 与测试端口；不会读取、复制、修改或删除真实
# ~/.claude-science。真实实例只通过 lsof 记录 8765 的监听 PID，并在每个
# 阶段比较是否保持不变。
set -euo pipefail

PROJ="$(cd "$(dirname "$0")/.." && pwd)"
TEST_ROOT="${CSSWITCH_REAL_TEST_ROOT:-${TMPDIR:-/tmp}/csswitch-real-machine-${UID}}"
TEST_HOME="$TEST_ROOT/home"
STATE_DIR="$TEST_ROOT/state"
BASELINE="$STATE_DIR/port-8765.pids"
PROXY_PORT="${CSSWITCH_TEST_PROXY_PORT:-18991}"
SANDBOX_PORT="${CSSWITCH_TEST_SANDBOX_PORT:-8990}"
# 平台检测：默认 Science 二进制路径
OS="$(uname -s)"
if [[ "$OS" == "Darwin" ]]; then
  DEFAULT_SCIENCE_BIN="/Applications/Claude Science.app/Contents/Resources/bin/claude-science"
else
  DEFAULT_SCIENCE_BIN="$HOME/.local/bin/claude-science"
fi
SCIENCE_BIN="${SCIENCE_BIN:-$DEFAULT_SCIENCE_BIN}"

die() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }

# 隔离护栏（修 GPT 三轮 P1）：拒绝把「我们创建的」隔离目录预置成符号链接。防有人先把 TEST_HOME
# 软链到真实 HOME，导致 prepare-legacy 经软链覆写真实 ~/.csswitch/config.json（或 chmod 真实目录）。
# 只查我们掌控的目录本身，不查 /var 这类系统父级软链（macOS 默认 TMPDIR 就在 /var/folders 下）。
reject_symlinks() {
  local p
  for p in "$@"; do
    if [ -L "$p" ]; then
      die "隔离路径是符号链接，拒绝（防指向真实目录）：$p"
    fi
  done
}

# canonicalize 隔离目录：解析所有父级软链后，测试 HOME 不得等于或位于真实 HOME 之内，
# 否则会往用户真实主目录里写测试配置。默认 TEST_ROOT 在 TMPDIR 下、天然通过。
assert_isolated_from_real_home() {
  local dir="$1" real_home canon
  real_home="$(cd "$HOME" 2>/dev/null && pwd -P)" || die "无法解析真实 HOME"
  canon="$(cd "$dir" 2>/dev/null && pwd -P)" || die "无法解析隔离目录：$dir"
  case "$canon/" in
    "$real_home/" | "$real_home"/*)
      die "隔离目录解析后落在真实 HOME 内，拒绝：${canon}（真实 HOME=${real_home}）。请把 CSSWITCH_REAL_TEST_ROOT 指向 HOME 之外（默认 TMPDIR 即可）。"
      ;;
  esac
}

# 安全写入状态/配置文件（修 GPT 四轮 P1）：先删掉目标处任何预置软链（`rm -f` 删的是链本身、
# 不跟随），再从 stdin 写一个全新普通文件。杜绝「`>` 跟随预置软链把内容写进真实文件」（如真实
# ~/.csswitch/config.json、或 STATE_DIR 里被软链的 port-8765.now）。**注意：这消除的是「跟随预置
# 软链」，非原子、不宣称消除检查与写入之间的并发竞态**（本地测试助手，非对抗实时攻击者的威胁模型）。
write_fresh() {
  local target="$1"
  if [ -d "$target" ] && [ ! -L "$target" ]; then
    die "目标是目录，拒绝当文件覆盖：$target"
  fi
  rm -f "$target"
  cat >"$target"
}

validate_ports() {
  case "$PROXY_PORT:$SANDBOX_PORT" in
    *[!0-9:]*|:*) die "测试端口必须是整数" ;;
  esac
  [ "$PROXY_PORT" -ne 8765 ] || die "代理端口命中真实实例保留端口 8765"
  [ "$SANDBOX_PORT" -ne 8765 ] || die "沙箱端口命中真实实例保留端口 8765"
  [ "$PROXY_PORT" -ne "$SANDBOX_PORT" ] || die "代理端口与沙箱端口不能相同"
}

listener_pids() {
  lsof -nP -t -iTCP:"$1" -sTCP:LISTEN 2>/dev/null | sort -u || true
}

assert_real_unchanged() {
  [ -f "$BASELINE" ] || die "缺少 8765 基线；先运行 preflight"
  local now="$STATE_DIR/port-8765.now"
  listener_pids 8765 | write_fresh "$now" # 删任何预置软链后写全新文件，绝不经软链写真实文件
  if ! cmp -s "$BASELINE" "$now"; then
    echo "基线 PID: $(tr '\n' ' ' <"$BASELINE")" >&2
    echo "当前 PID: $(tr '\n' ' ' <"$now")" >&2
    die "真实 Science 8765 监听发生变化，立即停止验收"
  fi
  pass "真实 Science 8765 监听 PID 保持不变"
}

assert_port_free() {
  local p="$1"
  [ -z "$(listener_pids "$p")" ] || die "测试端口 $p 已被占用"
}

preflight() {
  validate_ports
  reject_symlinks "$TEST_ROOT" "$TEST_HOME" "$STATE_DIR" # mkdir/chmod 前：拒绝预置软链
  umask 077
  mkdir -p "$TEST_HOME" "$STATE_DIR"
  chmod 700 "$TEST_ROOT" "$TEST_HOME" "$STATE_DIR"
  assert_isolated_from_real_home "$TEST_HOME" # mkdir 后：解析父链，确认不在真实 HOME 内
  listener_pids 8765 | write_fresh "$BASELINE" # 同理：删软链后写全新基线文件
  assert_port_free "$PROXY_PORT"
  assert_port_free "$SANDBOX_PORT"
  [ -x "$SCIENCE_BIN" ] || die "未找到可执行 Science：$SCIENCE_BIN"
  [ -x "$PROJ/desktop/src-tauri/target/release/desktop" ] || \
    echo "WARN: release 测试二进制尚未构建"
  pass "测试 HOME 已隔离：$TEST_HOME"
  pass "测试端口空闲：$PROXY_PORT / $SANDBOX_PORT"
  assert_real_unchanged
}

prepare_legacy() {
  validate_ports
  [ -f "$BASELINE" ] || die "先运行 preflight"
  [ -n "${DEEPSEEK_API_KEY:-}" ] || die "DEEPSEEK_API_KEY 未设置"
  [ -n "${DASHSCOPE_API_KEY:-}" ] || die "DASHSCOPE_API_KEY 未设置"
  command -v jq >/dev/null 2>&1 || die "prepare-legacy 需要 jq"
  # 写盘前再验隔离目录（含 STATE_DIR）都不是软链：这一步缩小 preflight 之后被换软链的窗口，并给出
  # 清晰早失败；真正的写安全由下面 write_fresh（删软链再写全新文件）保证，故不宣称消除竞态。
  reject_symlinks "$TEST_ROOT" "$TEST_HOME" "$STATE_DIR" "$TEST_HOME/.csswitch"
  assert_isolated_from_real_home "$TEST_HOME"
  local cfg_dir="$TEST_HOME/.csswitch"
  umask 077
  mkdir -p "$cfg_dir"
  chmod 700 "$cfg_dir"
  # config.json 本身若被预置成软链（指向真实 ~/.csswitch/config.json），write_fresh 会先删掉该软链
  # 再写全新普通文件，绝不经软链覆写真实配置。
  jq -n \
    --arg deepseek "$DEEPSEEK_API_KEY" \
    --arg qwen "$DASHSCOPE_API_KEY" \
    --argjson proxy_port "$PROXY_PORT" \
    --argjson sandbox_port "$SANDBOX_PORT" \
    '{provider:"deepseek",proxy_port:$proxy_port,sandbox_port:$sandbox_port,secret:"",mode:"proxy",providers:{deepseek:{key:$deepseek,base_url:"https://api.deepseek.com/anthropic",model:""},qwen:{key:$qwen,base_url:"https://dashscope.aliyuncs.com/compatible-mode/v1",model:"qwen3-max"}}}' \
    | write_fresh "$cfg_dir/config.json"
  chmod 600 "$cfg_dir/config.json"
  pass "已在独立测试 HOME 写入 v1 迁移样本（key 未回显）"
}

assert_running() {
  assert_real_unchanged
  [ -n "$(listener_pids "$PROXY_PORT")" ] || die "代理端口 $PROXY_PORT 未监听"
  [ -n "$(listener_pids "$SANDBOX_PORT")" ] || die "沙箱端口 $SANDBOX_PORT 未监听"
  local sbx_home="$TEST_HOME/.csswitch/sandbox/home"
  local data_dir="$sbx_home/.claude-science"
  local out
  out="$(HOME="$sbx_home" "$SCIENCE_BIN" status --data-dir "$data_dir" 2>/dev/null || true)"
  case "$out" in
    *'"running":true'*|*'"running": true'*) pass "独立 data-dir 的沙箱身份确认通过" ;;
    *) die "测试端口虽监听，但独立 data-dir 未报告 running=true" ;;
  esac
  pass "代理与沙箱测试端口均在监听"
}

assert_stopped() {
  assert_real_unchanged
  assert_port_free "$PROXY_PORT"
  assert_port_free "$SANDBOX_PORT"
  pass "测试代理与沙箱均已停止"
}

show_env() {
  cat <<EOF
CSSWITCH_REAL_TEST_ROOT=$TEST_ROOT
HOME=$TEST_HOME
CSSWITCH_REPO=$PROJ
CSSWITCH_TEST_PROXY_PORT=$PROXY_PORT
CSSWITCH_TEST_SANDBOX_PORT=$SANDBOX_PORT
EOF
}

case "${1:-}" in
  preflight) preflight ;;
  prepare-legacy) prepare_legacy ;;
  guard) assert_real_unchanged ;;
  assert-running) assert_running ;;
  assert-stopped) assert_stopped ;;
  env) show_env ;;
  *)
    echo "usage: $0 {preflight|prepare-legacy|guard|assert-running|assert-stopped|env}" >&2
    exit 2
    ;;
esac
