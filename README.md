# SMIUSB Open

Open tooling, a patched EVDI kernel bridge, and a Rust clean-room driver for
Silicon Motion SM768 USB displays (`090c:0768`).

## What already works

The included `libsmiusb_guard.so` prevents the confirmed USB-disconnect crash,
which was initially attributed to KVM switching. It interposes libusb hotplug
and transfer APIs. On an SM768 detach or a transfer
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

Forensics now show that the kernel panic itself involved no USB disconnect or
reset. EVDI powered on and set `1600x900@60`; 102 ms later GNOME Shell hit the
dma-buf BUG while closing a GEM handle. Together with the user's recollection,
that makes session unlock/DPMS wake the most likely trigger, although the
journal contains no explicit unlock signal.

Separate post-reboot validation on 2026-09-02 confirmed that kernel
`7.0.0-30` loaded EVDI `1.14.16-smiusb2`. At 21:38:16 the user manually
disconnected and reconnected the USB adapter after being asked to exercise the
then-suspected KVM path. The guard exited cleanly from
`libusb_interrupt_transfer`; systemd restarted the vendor service about 0.53
seconds later (`NRestarts=1`). The
adapter re-enumerated from USB address 3 to 5, traffic resumed after about 6.91
seconds, and `1600x900@60` was restored. That physical detach/reconnect cycle
produced no kernel BUG, Oops, panic, or coredump. It validates one USB-detach
cycle only: it did not exercise a KVM or session lock/unlock. Locking and
unlocking the session is now the suspected kernel-panic trigger and still needs
an instrumented reproduction, followed by repeated reconnect and suspend
stress testing.

The independent replacement now lives under [`rust/`](rust/). It has a
std-only libusb FFI layer with RAII ownership, passive hotplug observation,
strict protocol decoders, deterministic packet builders, and an offline frame
pipeline. A real reconnect/frame trace now confirms the outer frame layout,
but USB transmission remains locked out until the captured attach, mode-setting,
metadata, and acknowledgement semantics have been decoded and replayed safely;
the proprietary service still drives the monitor in the meantime. The staged
design and remaining validation gates are in
[the open-driver roadmap](docs/open-driver-roadmap.md).

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

Decode one extracted packet without transmitting it. `-` reads bounded hex
text from standard input; frame output contains only structural lengths and the
sequence byte, not display contents:

```sh
./rust/target/release/smiusbd-rs --decode-hex-file frame.hex
./rust/target/release/smiusbd-rs --decode-hex-file - < frame.hex
```

Capture an instrumented trigger window (default: 90 seconds; the script
auto-detects the bus when exactly one SM768 is present). For the next
kernel-panic test, lock and unlock the session while the capture is running,
without disconnecting the USB adapter:

```sh
./scripts/capture-kvm-cycle.sh 90 session
```

The script retains its legacy KVM-oriented filename for compatibility; the
capture itself is trigger-agnostic. Use `usb` instead of `session` only for an
explicit physical detach/reconnect test. A numerical bus may still be supplied
as argument 2 for compatibility, with the trigger as argument 3.

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
must pass offline decoding, replay, and repeated hardware validation before it
is allowed to drive the hardware.
