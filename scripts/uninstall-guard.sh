#!/bin/sh
set -eu

sudo rm -f /etc/systemd/system/smiusbdisplay.service.d/override.conf
sudo rm -f /usr/local/lib/smiusb/libsmiusb_guard.so
sudo systemctl daemon-reload
sudo systemctl restart smiusbdisplay.service

printf '%s\n' 'SMIUSB guard removed; vendor service restored.'
