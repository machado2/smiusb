# SMIUSB Open

Open tooling, a patched EVDI kernel bridge, and a Rust clean-room driver for
Silicon Motion SM768 USB displays (`090c:0768`).

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

The 2026-09-02 kernel panic is a separate dma-buf lifetime failure immediately
after EVDI enabled the USB display. Exact kernel disassembly found a
`vmapping_counter` remainder of one. The strongest actionable candidate is an
unlocked EVDI persistent mapping, so the open patch serializes that path and
prevents two concurrent callers from recording only one of two mappings. See
[the incident analysis](docs/kernel-panic-2026-09-02.md).

The independent replacement now lives under [`rust/`](rust/). It has a
std-only libusb FFI layer with RAII ownership, passive hotplug observation,
strict protocol decoders, deterministic packet builders, and an offline frame
pipeline. USB transmission remains locked out until attach, mode-setting, and
real frame captures are complete; the proprietary service still drives the
monitor in the meantime. The staged design and remaining validation gates are
in [the open-driver roadmap](docs/open-driver-roadmap.md).

## Build and test

```sh
meson setup build -Db_lundef=false
meson compile -C build
meson test -C build --print-errorlogs

cd rust
cargo test --offline
cargo build --release --offline
sudo install -m 0755 target/release/smiusbd-rs /usr/local/bin/smiusbd-open
```

The installed `smiusbd-open` command is currently a passive development tool:
it can observe reconnects and exercise the offline frame/JPEG pipeline, but it
cannot claim the adapter or replace the active display service yet.

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

## Install the EVDI panic fix

The installer verifies and patches the exact SMI EVDI source already present
on this host, builds it with DKMS, and checks the selected and initramfs copies
against the build artifact. It does not stop the service or unload the
currently active module, so the monitor stays on.

```sh
./scripts/install-patched-evdi.sh
```

The corrected module becomes active at the next normal reboot. Roll back the
on-disk module without interrupting the current session with:

```sh
./scripts/rollback-patched-evdi.sh
```

## Inspect and capture the device

Print all configurations, interfaces, alternate settings, and endpoints:

```sh
./build/smiusb-probe
./build/smiusb-probe --watch 120
./build/smiusbd --observe --duration 120
./rust/target/release/smiusbd-rs --observe --duration 120
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

The userspace repository is MIT-licensed; the EVDI-derived patch under
`kernel/evdi` is GPL-2.0-only. The project contains no Silicon Motion binary,
firmware, core dump, or copied decompiler output. The guard is a practical
intermediate fix; the replacement display transport is still experimental and
must be validated against usbmon captures before it is allowed to drive the
hardware.
