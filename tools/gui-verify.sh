#!/bin/sh
# End-to-end Ghostty GUI verification for hc-tui using nested isolated scratch
# hc sessions (outer hc drives hc-tui via `hc keys`/`hc send`; Ghostty attaches
# to the outer terminal and is captured via `screencapture` with zero macOS
# Accessibility permissions required).
#
# Never touches real hc sessions or user config.
#
# Usage:
#   tools/gui-verify.sh [screenshot-dir]
set -eu

SNAP_DIR="${1:-$(mktemp -d /tmp/hctui-gui-snaps-XXXXXX)}"
HC="${HC_BIN:-$(command -v hc)}"
TUI="${HC_TUI_BIN:-$PWD/target/release/hc-tui}"
GHOSTTY="${GHOSTTY_BIN:-/Applications/Ghostty.app/Contents/MacOS/ghostty}"

if [ ! -x "$HC" ]; then
    echo "hc binary not found (set HC_BIN)" >&2
    exit 1
fi
if [ ! -x "$TUI" ]; then
    echo "hc-tui binary not found at $TUI (build --release or set HC_TUI_BIN)" >&2
    exit 1
fi
if [ ! -x "$GHOSTTY" ]; then
    echo "Ghostty binary not found at $GHOSTTY (set GHOSTTY_BIN)" >&2
    exit 1
fi

ROOT=$(mktemp -d /tmp/hctui-gui-XXXXXX)
mkdir -p "$SNAP_DIR" \
    "$ROOT/in/home/work" "$ROOT/in/home/second" "$ROOT/in/state" "$ROOT/in/config" "$ROOT/in/run" \
    "$ROOT/out/home" "$ROOT/out/state" "$ROOT/out/config" "$ROOT/out/run"

in_hc() {
    env -u HC_TERMINAL_ID -u HERDR_ENV \
        HOME="$ROOT/in/home" XDG_STATE_HOME="$ROOT/in/state" XDG_CONFIG_HOME="$ROOT/in/config" \
        XDG_RUNTIME_DIR="$ROOT/in/run" HC_SESSION="hctui-gui-in-$$" HC_SOCKET="$ROOT/in/hc.sock" \
        "$HC" "$@"
}

out_hc() {
    env -u HC_TERMINAL_ID -u HERDR_ENV \
        HOME="$ROOT/out/home" XDG_STATE_HOME="$ROOT/out/state" XDG_CONFIG_HOME="$ROOT/out/config" \
        XDG_RUNTIME_DIR="$ROOT/out/run" HC_SESSION="hctui-gui-out-$$" HC_SOCKET="$ROOT/out/hc.sock" \
        "$HC" "$@"
}

cleanup() {
    if [ -n "${GHOSTTY_PID:-}" ]; then kill "$GHOSTTY_PID" 2>/dev/null || true; fi
    out_hc --json server stop >/dev/null 2>&1 || true
    in_hc --json server stop >/dev/null 2>&1 || true
    if [ -n "${OUT_PID:-}" ]; then kill "$OUT_PID" 2>/dev/null || true; fi
    if [ -n "${IN_PID:-}" ]; then kill "$IN_PID" 2>/dev/null || true; fi
    rm -rf "$ROOT"
}
trap cleanup EXIT

in_hc serve >"$ROOT/in.log" 2>&1 &
IN_PID=$!
out_hc serve >"$ROOT/out.log" 2>&1 &
OUT_PID=$!
for _ in $(seq 1 50); do
    [ -S "$ROOT/in/hc.sock" ] && [ -S "$ROOT/out/hc.sock" ] && break
    sleep 0.1
done

in_hc run --bg --cwd "$ROOT/in/home/work" --label herdr.space=work --label herdr.tab=tab1 \
    -- sh -c 'echo "work workspace live"; sleep 120' >/dev/null
sleep 0.3

cat >"$ROOT/run-tui.sh" <<EOF
#!/bin/sh
unset HC_TERMINAL_ID HERDR_ENV
export HOME="$ROOT/in/home"
export XDG_STATE_HOME="$ROOT/in/state"
export XDG_CONFIG_HOME="$ROOT/in/config"
export XDG_RUNTIME_DIR="$ROOT/in/run"
export HC_SESSION="hctui-gui-in-$$"
export HC_SOCKET="$ROOT/in/hc.sock"
exec "$TUI"
EOF
chmod +x "$ROOT/run-tui.sh"

cat >"$ROOT/attach-outer.sh" <<EOF
#!/bin/sh
unset HC_TERMINAL_ID HERDR_ENV
export HOME="$ROOT/out/home"
export XDG_STATE_HOME="$ROOT/out/state"
export XDG_CONFIG_HOME="$ROOT/out/config"
export XDG_RUNTIME_DIR="$ROOT/out/run"
export HC_SESSION="hctui-gui-out-$$"
export HC_SOCKET="$ROOT/out/hc.sock"
exec "$HC" attach t1
EOF
chmod +x "$ROOT/attach-outer.sh"

out_hc run --bg --cwd "$ROOT/out/home" -- "$ROOT/run-tui.sh" >/dev/null
sleep 0.5

"$GHOSTTY" \
    --title="hc-tui-e2e-gui" \
    --window-width=120 \
    --window-height=36 \
    --command="$ROOT/attach-outer.sh" >/dev/null 2>&1 &
GHOSTTY_PID=$!

sleep 2.0
/usr/sbin/screencapture -x "$SNAP_DIR/1-initial.png"
count_inner() {
    in_hc --json ls | python3 -c 'import sys,json; d=json.load(sys.stdin); print(len(d if isinstance(d,list) else d.get("terminals",[])))'
}

C1=$(count_inner)
[ "$C1" -eq 1 ] || { echo "expected 1 initial terminal, got $C1" >&2; exit 1; }

# Create workspace via Ctrl+B Shift+N -> clear -> type second folder
out_hc keys t1 ctrl+b shift+n >/dev/null
sleep 0.5
/usr/sbin/screencapture -x "$SNAP_DIR/2-create-dialog.png"
out_hc keys t1 ctrl+c >/dev/null
sleep 0.1
out_hc send t1 "$ROOT/in/home/second" >/dev/null
sleep 1.2
/usr/sbin/screencapture -x "$SNAP_DIR/3-created.png"
C2=$(count_inner)
[ "$C2" -eq 2 ] || { echo "expected 2 terminals after create, got $C2" >&2; exit 1; }

# Delete focused workspace via Ctrl+B Shift+D -> Enter
out_hc keys t1 ctrl+b shift+d >/dev/null
sleep 0.5
out_hc keys t1 enter >/dev/null
sleep 1.2
/usr/sbin/screencapture -x "$SNAP_DIR/4-deleted.png"
C3=$(count_inner)
[ "$C3" -eq 1 ] || { echo "expected 1 terminal after delete, got $C3" >&2; exit 1; }

# Detach via Ctrl+B q
out_hc keys t1 ctrl+b q >/dev/null
sleep 1.0
STATE=$(out_hc --json get t1 | python3 -c 'import sys,json; d=json.load(sys.stdin); print(str(d.get("state")) + ":" + str(d.get("exit_code")))')
[ "$STATE" = "finished:0" ] || { echo "expected clean detach finished:0, got $STATE" >&2; exit 1; }

echo "GUI verify OK (create 1->2, delete 2->1, detach finished:0). Screenshots in: $SNAP_DIR"
