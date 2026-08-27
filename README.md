# SMIUSB Open

Open tooling and a crash guard for Silicon Motion SM768 USB displays
(`090c:0768`), plus the clean-room foundation for a complete userspace driver.

## What already works

The included `libsmiusb_guard.so` prevents the confirmed KVM disconnect crash.
It interposes libusb hotplug and transfer APIs. On an SM768 detach or a transfer
returning `LIBUSB_ERROR_NO_DEVICE`, it exits the daemon before the unsafe vendor
cleanup runs; systemd starts a fresh instance after 500 ms. Outside the vendor
service, its default behavior remains converting a null handle into
`LIBUSB_ERROR_NO_DEVICE`. It does not modify the vendor binary or firmware.

The crash was reproduced from a real core dump: a detached initialization
thread called `libusb_bulk_transfer` with a null handle while libusb's hotplug
thread destroyed the same device. Detailed evidence and protocol observations
are in [docs/reverse-engineering.md](docs/reverse-engineering.md).

`smiusbd --observe` is the connection-lifecycle core of the independent
replacement. Its hotplug callback only enqueues referenced devices; one owned
worker opens and closes sessions, and every session has a generation number.
It deliberately does not claim the display interface or transmit provisional
protocol packets yet.

## Build and test

```sh
meson setup build -Db_lundef=false
meson compile -C build
meson test -C build --print-errorlogs
```

`-Db_lundef=false` is required because the guard resolves the real libusb
functions from the next object in the dynamic linker's search order.

## Install the immediate guard

```sh
./scripts/install-guard.sh
```

This installs the library under `/usr/local/lib/smiusb`, adds a systemd drop-in
for `smiusbdisplay.service`, and restarts the service. The drop-in enables the
restart fence and uses `SIGKILL` for explicit service stops, avoiding the same
unsafe vendor destructors during shutdown. To revert:

```sh
./scripts/uninstall-guard.sh
```

Guard activity is visible with:

```sh
journalctl -u smiusbdisplay.service -f
```

## Inspect and capture the device

Print all configurations, interfaces, alternate settings, and endpoints:

```sh
./build/smiusb-probe
./build/smiusb-probe --watch 120
./build/smiusbd --observe --duration 120
```

Capture one KVM away/back cycle (default: USB bus 1 for 90 seconds):

```sh
./scripts/capture-kvm-cycle.sh 90 1
```

On Ubuntu 26.04 the capture script temporarily places only the packaged
`tshark` AppArmor profile in complain/audit mode because that profile omits
`/dev/usbmon*`. A trap restores enforcing mode on success, error, or signal.

Captures and crash artifacts are intentionally ignored by Git. They can
contain display contents or proprietary firmware traffic and must be reviewed
before sharing. When the target address can be identified, the capture script
automatically discards packets from other devices on the same USB bus.

## Project boundaries

The repository is MIT-licensed and contains no Silicon Motion binary,
firmware, core dump, or copied decompiler output. The guard is a practical
intermediate fix; the replacement display transport is still experimental and
must be validated against usbmon captures before it is allowed to drive the
hardware.
