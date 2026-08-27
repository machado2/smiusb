#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
duration=${1:-90}
bus=${2:-1}
timestamp=$(date +%Y%m%d-%H%M%S)
capture_dir="$project_dir/captures"
capture_file="$capture_dir/smiusb-kvm-$timestamp.pcapng"
summary_file="$capture_dir/smiusb-kvm-$timestamp.txt"
capture_tmp=''
filtered_tmp=''
restore_apparmor=0

cleanup()
{
    if [ -n "$capture_tmp" ]; then
        sudo rm -f -- "$capture_tmp"
    fi
    if [ -n "$filtered_tmp" ]; then
        sudo rm -f -- "$filtered_tmp"
    fi
    if [ "$restore_apparmor" -eq 1 ]; then
        sudo aa-enforce /etc/apparmor.d/tshark >/dev/null
    fi
}
trap cleanup EXIT HUP INT TERM

case "$duration" in
    *[!0-9]*|'')
        printf '%s\n' 'duration must be a positive number of seconds' >&2
        exit 2
        ;;
esac
if [ "$duration" -eq 0 ]; then
    printf '%s\n' 'duration must be a positive number of seconds' >&2
    exit 2
fi
case "$bus" in
    *[!0-9]*|'')
        printf '%s\n' 'bus must be a positive USB bus number' >&2
        exit 2
        ;;
esac
if [ "$bus" -eq 0 ]; then
    printf '%s\n' 'bus must be a positive USB bus number' >&2
    exit 2
fi

mkdir -p "$capture_dir"
sudo modprobe usbmon
capture_tmp=$(sudo mktemp /tmp/smiusb-kvm.XXXXXX.pcapng)
initial_addresses=$(lsusb -d 090c:0768 2>/dev/null | awk -v wanted="$(printf '%03d' "$bus")" '
    $2 == wanted { gsub(":", "", $4); print $4 }
')

# Ubuntu 26.04's tshark AppArmor child profile does not allow /dev/usbmon*.
# Relax only that profile for the bounded capture and restore it in cleanup().
if command -v aa-status >/dev/null 2>&1 && \
    sudo aa-status --json 2>/dev/null | \
        jq -e '.profiles.tshark == "enforce"' >/dev/null; then
    sudo aa-complain /etc/apparmor.d/tshark >/dev/null
    restore_apparmor=1
fi

printf 'Capturing usbmon%s for %s seconds. Switch the KVM away and back now.\n' \
    "$bus" "$duration"
sudo tshark -q -i "usbmon$bus" -a "duration:$duration" -w "$capture_tmp"

observed_addresses=$(sudo tshark -r "$capture_tmp" \
    -Y "usb.bus_id == $bus && usb.idVendor == 0x090c && usb.idProduct == 0x0768" \
    -T fields -e usb.device_address 2>/dev/null || true)
target_addresses=$(printf '%s\n%s\n' "$initial_addresses" "$observed_addresses" | \
    awk '/^[0-9]+$/ { print $1 + 0 }' | sort -nu)

if [ -n "$target_addresses" ]; then
    target_filter=''
    for address in $target_addresses; do
        if [ -n "$target_filter" ]; then
            target_filter="$target_filter || "
        fi
        target_filter="${target_filter}usb.device_address == $address"
    done
    filtered_tmp=$(sudo mktemp /tmp/smiusb-target.XXXXXX.pcapng)
    sudo tshark -r "$capture_tmp" -Y "$target_filter" -w "$filtered_tmp" >/dev/null 2>&1
    sudo chown "$(id -u):$(id -g)" "$filtered_tmp"
    mv "$filtered_tmp" "$capture_file"
    filtered_tmp=''
    sudo rm -f -- "$capture_tmp"
    capture_tmp=''
    printf 'Kept only SMIUSB device address(es): %s\n' \
        "$(printf '%s' "$target_addresses" | tr '\n' ' ')"
else
    sudo chown "$(id -u):$(id -g)" "$capture_tmp"
    mv "$capture_tmp" "$capture_file"
    capture_tmp=''
    printf '%s\n' \
        'Warning: target address not found; capture contains the full USB bus.' >&2
fi

tshark -r "$capture_file" \
    -Y 'usb' \
    -T fields \
    -E header=y \
    -E separator=, \
    -e frame.number \
    -e frame.time_relative \
    -e usb.bus_id \
    -e usb.device_address \
    -e usb.endpoint_address \
    -e usb.transfer_type \
    -e usb.urb_type \
    -e usb.data_len \
    -e usb.capdata > "$summary_file"

printf 'Capture: %s\nSummary: %s\n' "$capture_file" "$summary_file"
