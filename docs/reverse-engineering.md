# Reverse-engineering notes: Silicon Motion SM768

These notes describe behavior independently observed on a locally installed
Silicon Motion Linux driver. The repository does not contain or redistribute
the vendor daemon, firmware, core dumps, or decompiler output.

## Target

- USB identity: `090c:0768`
- Product string: `SMI USB Display`
- Silicon Motion family reported by the daemon: `SM768`
- Vendor daemon: `SMIUSBDisplayManager` v2.24.8.0
- USB library bundled by the vendor: libusb 1.0.26
- USB library supplied by Ubuntu 26.04: libusb 1.0.29

## Confirmed disconnect race

The 2026-08-24 09:39:19 crash has two relevant threads:

1. The libusb event thread is inside the hotplug-detach callback. It removes
   the `SMIDev` from a global map and runs `SMIDev::~SMIDev()`.
2. A detached initialization thread is still executing
   `SMIDisplayClass768::initCmds()`.
3. The destructor clears the device state while the initialization thread
   continues through its raw pointer.
4. That thread calls `libusb_bulk_transfer(NULL, 0x02, ..., 48, ..., 1000)`.
5. libusb 1.0.26 dereferences the handle mutex at offset `0x14c`, producing
   SIGSEGV at address `0x14c`.

This is a daemon lifetime bug, not an EVDI or kernel crash. Replacing libusb
alone is insufficient because the invalid argument crosses libusb's public
API boundary before the crash.

## Initial SM768 protocol observations

These fields are provisional until checked against usbmon captures.

- Display traffic uses bulk transfers; the failing initialization packet used
  endpoint `0x02`, a 48-byte buffer, and a 1000 ms timeout.
- The wire prefix is the 12-byte ASCII marker `smifalconsta`.
- The initial capabilities request is 48 bytes and is followed by a response
  associated with command/data type `0x65`.
- A frame packet starts with a 48-byte transport header, followed by a
  `0x5000`-byte JPEG decoder header and then the compressed payload.
- The total frame transfer is padded so its transfer length is not an exact
  multiple of 512 bytes.
- Frames are JPEG-based. Dirty rectangles, frame indices, target/source IDs,
  cursor commands, EDID, power state, and heartbeat are separate protocol
  operations.

## Live USB capture, 2026-08-27

A 10-second usbmon capture with the display connected produced 26,149 packets.
Filtering it to `090c:0768` confirmed the complete USB layout:

- Configuration 1 exposes five interfaces. Display transport is interface 4,
  alternate setting 1.
- The transport endpoints are interrupt IN `0x81` (1024-byte max packet),
  interrupt OUT `0x01` (1024-byte max packet), and bulk OUT `0x02` (512-byte
  max packet).
- With a static desktop, endpoint `0x02` sent one 44-byte `smifalconsta` packet
  per second. Its little-endian word at offset 12 is also 44.
- Endpoint `0x81` completed a 1024-byte read roughly twice per second. Its
  payload starts with the same marker and contains the active 1600x900 mode
  followed by a table of supported resolutions and refresh rates.
- No full JPEG frame was present during this static interval. A capture that
  includes visible screen changes is still needed to validate frame chunking.

The capture remains under the ignored `captures/` directory because usbmon on
the whole bus also recorded camera traffic. It must not be published without
privacy review.

## Correct lifetime model for the replacement

The open implementation must follow these invariants:

- The libusb hotplug callback only takes a device reference and enqueues an
  event. It never joins threads, closes a handle, or destroys a session.
- Each connection has a monotonically increasing generation. Work from an old
  generation is rejected after reconnect.
- Disconnect first marks the session as stopping, then cancels transfers,
  drains callbacks, joins owned threads, releases interfaces, and finally
  closes the handle.
- No worker is detached. Every worker is owned and joined by its session.
- `LIBUSB_ERROR_NO_DEVICE` is a normal state transition, not a fatal error.

The first guard implemented the last invariant at the ABI boundary. Field
testing showed that returning `LIBUSB_ERROR_NO_DEVICE` was not sufficient: the
detached `initCmds` worker continued into `readIntQ`, looked up a device already
removed from the global map, received a null `shared_ptr`, and attempted to lock
its queue mutex at address `0x6f0`. Other runs aborted while destroying futexes
or closing an already-invalid libusb handle.

The current guard therefore treats the process as the only safe lifetime
boundary available without rewriting the vendor daemon. It intercepts the
SM768 `DEVICE_LEFT` callback and exits before vendor cleanup. It also exits if a
USB transfer observes `LIBUSB_ERROR_NO_DEVICE`. The systemd unit restarts a
clean process after 500 ms. This deliberately trades a short reconnect delay
for deterministic teardown of every vendor thread by the kernel.
