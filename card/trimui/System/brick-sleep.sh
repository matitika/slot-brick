#!/bin/sh
# Suspend-to-RAM for the TrimUI Brick, run by slot when it dozes (Power pressed). The Power key
# wakes the kernel, and slot sees the same press as its wake. Works on stock firmware and on
# Slot OS. Adapted from LordNordyx's Slot-Brick sleep.sh (github.com/LordNordyx/Slot-Brick,
# GPL-3.0), which follows NextUI's order. Everything changed for the sleep (radios, the
# mem_sleep mode, stock's input flag) is put back after.
set -u
ROOT=${SLOT_ROOT:-/mnt/SDCARD}
SYS=${SLOT_SLEEP_SYSROOT:-/sys}
mkdir -p "$ROOT/Logs"
LOG="$ROOT/Logs/sleep.log"
if [ -f "$LOG" ] && [ "$(wc -c < "$LOG")" -gt 131072 ]; then mv -f "$LOG" "$LOG.1"; fi
exec >> "$LOG" 2>&1
printf '\nsleep: %s\n' "$(date)"
BATTERY="$SYS/class/power_supply/axp2202-battery"
battery() {
    printf 'battery %s: %s%%' "$1" "$(cat "$BATTERY/capacity" 2>/dev/null || echo '?')"
    [ -r "$BATTERY/voltage_now" ] && printf ', %s mV' "$(( $(cat "$BATTERY/voltage_now") / 1000 ))"
    printf '\n'
}
battery before
[ -w "$SYS/power/state" ] && grep -qw mem "$SYS/power/state" || { echo 'no mem suspend'; exit 1; }

radios=''
mem_sleep_was=''
# On stock, trimui_inputd parks its polling while this flag exists.
INPUT_FLAG=/tmp/system_suspend
owned_flag=0

restore() {
    status=$?
    trap - EXIT HUP INT TERM
    [ "$owned_flag" = 0 ] || rm -f "$INPUT_FLAG"
    [ -z "$mem_sleep_was" ] || printf '%s\n' "$mem_sleep_was" > "$SYS/power/mem_sleep"
    for r in $radios; do printf '0\n' > "$r/soft" || echo "radio restore failed: $r"; done
    battery after
    echo "done: $status"
    exit "$status"
}
trap restore EXIT
trap 'exit 1' HUP INT TERM

if [ ! -e "$INPUT_FLAG" ]; then
    : > "$INPUT_FLAG" && owned_flag=1
fi
# Radios left powered still draw current asleep: block them, and unblock exactly those after.
for r in "$SYS"/class/rfkill/rfkill*; do
    [ -w "$r/soft" ] && [ "$(cat "$r/soft" 2>/dev/null)" = 0 ] || continue
    printf '1\n' > "$r/soft" && radios="$radios $r"
done
# Real suspend-to-RAM where the kernel offers a choice, not the costlier s2idle.
f="$SYS/power/mem_sleep"
if [ -w "$f" ]; then
    current=$(sed -n 's/.*\[\([a-z0-9]*\)\].*/\1/p' "$f")
    if [ -n "$current" ] && [ "$current" != deep ] && grep -qw deep "$f"; then
        printf 'deep\n' > "$f" && mem_sleep_was=$current
    fi
fi
sync
started=$(date +%s)
echo 'suspending'
printf 'mem\n' > "$SYS/power/state"
status=$?
elapsed=$(( $(date +%s) - started ))
echo "kernel returned $status after $elapsed s"
# Some tg5040-family kernels report an error despite having slept (as NextUI works around).
if [ "$status" -ne 0 ] && [ "$elapsed" -gt 5 ]; then status=0; fi
exit "$status"
