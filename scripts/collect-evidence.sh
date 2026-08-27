#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
timestamp=$(date +%Y%m%d-%H%M%S)
output_dir="$project_dir/artifacts/evidence-$timestamp"

mkdir -p "$output_dir"
lsusb -v -d 090c:0768 > "$output_dir/lsusb-090c-0768.txt" 2>&1 || true
"$project_dir/build/smiusb-probe" > "$output_dir/probe.txt" 2>&1 || true
journalctl -k -b --no-pager > "$output_dir/kernel-journal.txt"
journalctl -u smiusbdisplay.service -b --no-pager \
    > "$output_dir/vendor-service-journal.txt"
coredumpctl --no-pager list > "$output_dir/coredumps.txt" 2>&1 || true
systemctl cat smiusbdisplay.service > "$output_dir/service-definition.txt"

printf 'Evidence written to %s\n' "$output_dir"
