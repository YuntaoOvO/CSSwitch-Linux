#!/bin/bash
# 安装/卸载/查看 CSSwitch 每日维护巡检 systemd timer（Linux）。
#   scripts/install-maintenance-systemd.sh install     # 安装 user unit 并启用 timer
#   scripts/install-maintenance-systemd.sh uninstall   # 停用并删除
#   scripts/install-maintenance-systemd.sh status      # 查看 timer 状态与最近日志
#   scripts/install-maintenance-systemd.sh run         # 立刻手动触发一次
#
# systemd user unit 路径：~/.config/systemd/user/
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
UNIT_DIR="$HOME/.config/systemd/user"
SRC_SERVICE="$REPO/scripts/csswitch-maintenance.service"
SRC_TIMER="$REPO/scripts/csswitch-maintenance.timer"
DST_SERVICE="$UNIT_DIR/csswitch-maintenance.service"
DST_TIMER="$UNIT_DIR/csswitch-maintenance.timer"

# systemd service 文件里的 %h 指向 $HOME，但我们将脚本路径替换为绝对路径以保证准确性。
# 先检查是否在 Linux 上运行。
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "此脚本仅用于 Linux (systemd)。macOS 请使用 install-maintenance.sh (launchd)。" >&2
  exit 1
fi

SYSTEMCTL="systemctl --user"

cmd="${1:-status}"
case "$cmd" in
  install)
    mkdir -p "$UNIT_DIR" "$REPO/findings/auto-maint/logs"
    # 生成 service 文件：将脚本路径写绝对路径（替换 %h 占位）
    sed "s|%h/CSSwitch/scripts/daily-maintenance.sh|$REPO/scripts/daily-maintenance.sh|g" \
      "$SRC_SERVICE" | \
    sed "s|%h/CSSwitch/findings/|$REPO/findings/|g" \
      > "$DST_SERVICE"
    cp "$SRC_TIMER" "$DST_TIMER"
    $SYSTEMCTL daemon-reload
    $SYSTEMCTL enable --now csswitch-maintenance.timer
    echo "已安装并启用 systemd timer：csswitch-maintenance（每天 09:00 / 21:00）"
    $SYSTEMCTL status csswitch-maintenance.timer 2>/dev/null | head -10 || true
    ;;
  uninstall)
    $SYSTEMCTL disable --now csswitch-maintenance.timer 2>/dev/null || true
    $SYSTEMCTL stop csswitch-maintenance.service 2>/dev/null || true
    rm -f "$DST_SERVICE" "$DST_TIMER"
    $SYSTEMCTL daemon-reload
    echo "已停用并删除 systemd units：csswitch-maintenance"
    ;;
  status)
    if $SYSTEMCTL is-active csswitch-maintenance.timer >/dev/null 2>&1; then
      echo "== Timer 状态 =="
      $SYSTEMCTL status csswitch-maintenance.timer 2>/dev/null | head -15 || true
    else
      echo "== Timer 未加载 =="
    fi
    echo "== 最近报告 =="
    ls -t "$REPO/findings/auto-maint"/report-*.md 2>/dev/null | head -3 || echo "（还没有报告）"
    echo "== 最近日志 =="
    ls -t "$REPO/findings/auto-maint/logs"/run-*.log 2>/dev/null | head -3 || echo "（还没有日志）"
    ;;
  run)
    $SYSTEMCTL start csswitch-maintenance.service
    echo "已触发一次运行，几秒后看 findings/auto-maint/logs/ 与 report-*.md"
    ;;
  *)
    echo "用法：$0 {install|uninstall|status|run}" >&2
    exit 2
    ;;
esac
