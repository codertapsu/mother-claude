#!/usr/bin/env bash
#
# setup-walk/macos.sh — Phase 0 S5 on macOS: the setup checks of
# docs/PLAN-MACHINES.md §5.14 met in their worst states, then put back.
#
# Run it yourself in Terminal, as your own user (it calls sudo itself, only for
# the firewall). It asks before every change and restores every setting it
# changed — on success, on an error and on Ctrl-C:
#
#   bash spikes/machines/setup-walk/macos.sh            # the walk, about 5 minutes
#   bash spikes/machines/setup-walk/macos.sh run ARGS…  # `mc-spike ARGS…` inside the probe app
#
# 1. Local Network, as the app will meet it: "MC Spike Probe.app" is the spike
#    binary in a bundle whose Info.plist has NSLocalNetworkUsageDescription
#    (today's Mother Claude has none, and macOS refuses it without asking).
#    Does macOS ask now? Allowed, then switched off in System Settings, then on.
# 2. LAN discovery: mDNS between two endpoints inside the probe app.
# 3. The application firewall: on, with "Block all incoming connections" (sudo);
#    the checks read again; your settings back.
# 4. Sleep: pmset's timers, and a keep-awake assertion as pmset reports it.
#
# `run` is for the LAN test with Windows (RESULTS.md, "Still to run"): it runs
# the spike inside the probe app, which holds the Local Network permission.
# Results: spikes/machines/out/setup-walk-macos.txt (git-ignored).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MACHINES="$(cd "$HERE/.." && pwd)"
OUT="$MACHINES/out"
BIN="$MACHINES/mc-spike/target/release/mc-spike"
APP="$OUT/MC Spike Probe.app"
FW=/usr/libexec/ApplicationFirewall/socketfilterfw
LOG="$OUT/setup-walk-macos.txt"
mkdir -p "$OUT"

[ "$(id -u)" -ne 0 ] || { echo "Run this as your own user, not with sudo: it asks for sudo itself." >&2; exit 1; }
[ -x "$BIN" ] || {
  echo "Build the spike first: cargo build --release --manifest-path spikes/machines/mc-spike/Cargo.toml" >&2
  exit 1
}

log() { printf '%s\n' "$*" | tee -a "$LOG"; }
section() { printf '\n== %s\n' "$*" | tee -a "$LOG"; }

# The probe app. Rebuilt only when the spike binary changed: an ad-hoc
# signature changes with every build, and macOS may then ask again.
build_probe() {
  if [ -x "$APP/Contents/MacOS/mc-spike" ] && ! [ "$BIN" -nt "$APP/Contents/MacOS/mc-spike" ]; then
    return 0
  fi
  rm -rf "$APP"
  mkdir -p "$APP/Contents/MacOS"
  cp "$BIN" "$APP/Contents/MacOS/mc-spike"
  cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>dev.motherclaude.spike.probe</string>
  <key>CFBundleName</key><string>MC Spike Probe</string>
  <key>CFBundleExecutable</key><string>mc-spike</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSUIElement</key><true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>The Mother Claude Machines spike checks that it can reach your other computers on this network.</string>
  <key>NSBonjourServices</key>
  <array><string>_irohv1._udp</string></array>
</dict>
</plist>
PLIST
  codesign --force --sign - "$APP" >/dev/null 2>&1
}

# Runs `mc-spike ARGS…` inside the probe app (so macOS holds the app
# responsible), waits for it, and leaves its output in out/probe-NAME.{out,err}.
in_app() {
  local name="$1"; shift
  rm -f "$OUT/probe-$name.out" "$OUT/probe-$name.err"
  open -W -n "$APP" --stdout "$OUT/probe-$name.out" --stderr "$OUT/probe-$name.err" --args "$@"
}

# "allowed" or "refused (…)" from a `checks` output.
lan_state() {
  python3 -I - "$1" <<'PY'
import json, sys
try:
    n = json.load(open(sys.argv[1]))["network"]
except Exception as e:
    print(f"unreadable ({e})"); sys.exit()
g, m = n.get("sendToGateway") or {}, n.get("sendMdnsQuery") or {}
if g.get("ok") and m.get("ok"):
    print("allowed")
else:
    print(f"refused (gateway: {g.get('error', 'ok')}; mDNS: {m.get('error', 'ok')})")
PY
}

if [ "${1:-}" = run ]; then
  shift
  build_probe
  in_app run "$@"
  cat "$OUT/probe-run.out"
  cat "$OUT/probe-run.err" >&2
  exit 0
fi

: > "$LOG"
log "setup walk, macOS $(sw_vers -productVersion), $(date -u +%Y-%m-%dT%H:%MZ)"

# ---- 1. Local Network ---------------------------------------------------------
section "1. Local Network, as the app will meet it"
build_probe
in_app ln-first checks
log "first try from the probe app: $(lan_state "$OUT/probe-ln-first.out")"
echo
echo "macOS may be asking now: \"MC Spike Probe\" would like to find and connect to"
echo "devices on your local network. If it is, click Allow."
read -r -p "Did macOS ask? [y/n] " asked
log "macOS asked: ${asked:-?}"
in_app ln-allowed checks
log "after your answer: $(lan_state "$OUT/probe-ln-allowed.out")"

# ---- 2. LAN discovery -------------------------------------------------------------
section "2. LAN discovery (mDNS) inside the probe app"
in_app mdns local --only mdns --mib 16
python3 -I - "$OUT/probe-mdns.out" <<'PY' | tee -a "$LOG"
import json, sys
lines = [l for l in open(sys.argv[1]) if l.strip()]
if not lines:
    print("mdns: no result (see out/probe-mdns.err)")
for line in lines:
    r = json.loads(line)
    if "error" in r:
        print(f"mdns: failed: {r['error']}")
    else:
        s = r["suite"]
        print(f"mdns: connected in {r['connectMs']} ms by id alone; "
              f"raw {s['rawUpload']['MiBps']}/{s['rawDownload']['MiBps']} MiB/s")
PY

# ---- 1b. Switched off later -------------------------------------------------------
section "1b. Local Network switched off later, then on again"
open "x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork" || true
read -r -p "In System Settings > Privacy & Security > Local Network, switch \"MC Spike Probe\" OFF, then press Enter. " _
in_app ln-off checks
log "switched off: $(lan_state "$OUT/probe-ln-off.out")"
read -r -p "Switch it back ON (the LAN test with Windows uses it), then press Enter. " _
in_app ln-on checks
log "switched on again: $(lan_state "$OUT/probe-ln-on.out")"

# ---- 3. The application firewall --------------------------------------------------
section "3. The application firewall, on and blocking all incoming connections"
orig_global="$("$FW" --getglobalstate)"
orig_blockall="$("$FW" --getblockall)"
log "before: $orig_global / $orig_blockall"
FW_CHANGED=0
restore_fw() {
  [ "$FW_CHANGED" = 1 ] || return 0
  FW_CHANGED=0
  if grep -q 'disabled' <<<"$orig_blockall"; then sudo "$FW" --setblockall off >/dev/null; fi
  if grep -q 'State = 0' <<<"$orig_global"; then sudo "$FW" --setglobalstate off >/dev/null; fi
  log "firewall put back: $("$FW" --getglobalstate) / $("$FW" --getblockall)"
}
trap restore_fw EXIT
trap 'restore_fw; exit 130' INT TERM
read -r -p "Turn the firewall on with Block all for about a minute? Phones lose the LAN connection meanwhile. sudo asks your password. [y/N] " ok
if [ "${ok:-n}" = y ] || [ "${ok:-n}" = Y ]; then
  sudo -v
  FW_CHANGED=1
  sudo "$FW" --setglobalstate on >/dev/null
  sudo "$FW" --setblockall on >/dev/null
  "$BIN" checks > "$OUT/fw-blocked.json"
  python3 -I - "$OUT/fw-blocked.json" <<'PY' | tee -a "$LOG"
import json, sys
f = json.load(open(sys.argv[1]))["macosFirewall"]
print("read while blocked (standard user): " + " | ".join(f[k].get("out", f[k].get("err", "?")).replace("\n", " ") for k in ("globalState", "blockAll", "motherClaudeApp")))
PY
  restore_fw
else
  log "skipped"
fi

# ---- 4. Sleep ---------------------------------------------------------------------
section "4. Sleep"
log "pmset: $(pmset -g | grep -E '^ *(sleep|displaysleep|standby|powernap) ' | tr -s ' ' | tr '\n' ';')"
caffeinate -i -t 4 &
cpid=$!
sleep 1
log "with a keep-awake assertion held: $(pmset -g assertions | grep -m1 -E 'PreventUserIdleSystemSleep +[01]' | tr -s ' ')"
wait "$cpid" || true

section "Done"
log "Results: $LOG"
log "Your settings are as they were, except Local Network for \"MC Spike Probe\" (as you left it)."
