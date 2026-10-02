#!/bin/sh
# managed by vrspi; reinstalling the integration replaces this file.
# VRSPI_INTEGRATION_ID=qwen
# VRSPI_INTEGRATION_VERSION=1

[ "${1:-}" = "session" ] || exit 0
[ "${VRSPI_ENV:-${HERDR_ENV:-}}" = "1" ] || exit 0
[ -n "${VRSPI_PANE_ID:-${HERDR_PANE_ID:-}}" ] || exit 0
[ -n "${VRSPI_SOCKET_PATH:-${HERDR_SOCKET_PATH:-}}" ] || exit 0
if [ -n "${VRSPI_BIN_PATH:-${HERDR_BIN_PATH:-}}" ]; then
    [ -x "$HERDR_BIN_PATH" ] || exit 0
else
    command -v vrspi >/dev/null 2>&1 || exit 0
fi
command -v python3 >/dev/null 2>&1 || exit 0

python3 -c '
import json
import os
import subprocess
import sys
import time

try:
    payload = json.load(sys.stdin)
    session_id = payload.get("session_id")
    source = payload.get("source")
    if not isinstance(session_id, str) or not session_id:
        raise ValueError
    command = os.environ.get("HERDR_BIN_PATH") or "vrspi"
    args = [
        command, "pane", "report-agent-session", os.environ["HERDR_PANE_ID"],
        "--source", "vrspi:qwen", "--agent", "qwen",
        "--agent-session-id", session_id, "--seq", str(time.time_ns()),
    ]
    if source in ("startup", "resume", "clear", "compact", "branch"):
        args.extend(["--session-start-source", source])
    subprocess.run(
        args,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=1,
        check=False,
    )
except Exception:
    pass
' 2>/dev/null || true
