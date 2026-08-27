#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
build_dir="$project_dir/build"

if [ -d "$build_dir" ]; then
    meson setup --reconfigure "$build_dir" "$project_dir" \
        --prefix=/usr/local -Db_sanitize=none -Db_lundef=false
else
    meson setup "$build_dir" "$project_dir" \
        --prefix=/usr/local -Db_sanitize=none -Db_lundef=false
fi

meson compile -C "$build_dir"
meson test -C "$build_dir" --print-errorlogs
sudo meson install -C "$build_dir"
sudo install -D -m 0644 \
    "$project_dir/packaging/systemd/smiusbdisplay.service.d/override.conf" \
    /etc/systemd/system/smiusbdisplay.service.d/override.conf
sudo systemctl daemon-reload
sudo systemctl restart smiusbdisplay.service

printf '%s\n' 'SMIUSB guard installed and smiusbdisplay.service restarted.'
systemctl --no-pager --full status smiusbdisplay.service
