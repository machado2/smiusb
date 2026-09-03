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

The command observations below came from runtime behavior and binary analysis.
The frame-layout statements are updated with the 2026-09-02 wire capture where
noted.

- Display traffic uses bulk transfers; the failing initialization packet used
  endpoint `0x02`, a 48-byte buffer, and a 1000 ms timeout.
- The wire prefix is the 12-byte ASCII marker `smifalconsta`.
- The initial capabilities request is 48 bytes and is followed by a response
  associated with command/data type `0x65`.
- A frame packet starts with a 48-byte transport header, followed by the
  compressed JPEG bytes, exactly `0x5000` bytes of CnM metadata containing
  structured non-zero data, and zero padding. The JPEG SOI is at offset 48 and
  its end is determined by the structurally parsed EOI. The old assumption that
  decoder metadata preceded the JPEG was incorrect.
- In all 13 captured SuperSpeed frame samples, the transfer length was
  `48 + align_up(jpeg_length + 0x5000, 1024)`. The earlier `+1`/512-byte rule
  inferred from the vendor's internal allocation was not present on the wire;
  it must not be treated as a packet-length rule.
- The JPEG input format is BGRX with 4:2:2 chroma subsampling, matching
  TurboJPEG's `TJPF_BGRX` and `TJSAMP_422` values.
- Frames are JPEG-based. Dirty rectangles, frame indices, target/source IDs,
  cursor commands, EDID, power state, and heartbeat are separate protocol
  operations.

## Live USB capture, 2026-08-27

A 10-second usbmon capture with the display connected produced 26,149 packets.
Filtering it to `090c:0768` confirmed the complete USB layout:

- Configuration 1 exposes five interfaces. Display transport is interface 4,
  alternate setting 1.
- The transport endpoints are interrupt IN `0x81`, interrupt OUT `0x01`, and
  bulk OUT `0x02`. Max-packet size depends on negotiated USB speed: the older
  High Speed capture reports 512 bytes for bulk OUT, while the currently
  attached SuperSpeed device reports 1024. The replacement must read endpoint
  descriptors instead of hardcoding either value.
- With a static desktop, endpoint `0x02` sent one 44-byte `smifalconsta` packet
  per second. Its little-endian word at offset 12 is also 44. A passive uprobe
  later showed an ASCII decimal worker-thread tag beginning at offset 35 and a
  NUL terminator, followed by three bytes the vendor leaves uninitialized. The
  Rust replacement zeroes the unused bytes rather than leaking stack data.
- Endpoint `0x81` completed a 1024-byte read roughly twice per second. Its
  payload starts with the same marker and contains the active 1600x900 mode
  followed by a table of supported resolutions and refresh rates.
- No full JPEG frame was present during this static interval. A capture that
  includes visible screen changes is still needed to validate frame chunking.

The capture remains under the ignored `captures/` directory because usbmon on
the whole bus also recorded camera traffic. It must not be published without
privacy review.

## Post-reboot physical reconnect and frame capture, 2026-09-02

After reboot, kernel `7.0.0-30` was running EVDI `1.14.16-smiusb2`. At 21:38:16
the user manually disconnected and reconnected the USB adapter after being
asked to exercise the then-suspected KVM path. This was a physical USB
detach/reconnect, not a KVM switch or a session lock/unlock. It exercised the
guard and provided a limited smoke test of the patched EVDI path:

- the guard exited the vendor process cleanly from
  `libusb_interrupt_transfer` during detach;
- systemd restarted it about 0.53 seconds later and reported exactly one
  restart (`NRestarts=1`);
- the adapter changed USB address from 3 to 5, and captured traffic confirmed
  reattachment after about 6.91 seconds;
- the `1600x900@60` mode returned; and
- the physical reconnect produced no kernel BUG, Oops, panic, or userspace
  coredump.

This result must not be interpreted as reproducing or fixing the kernel panic.
The current suspected trigger is locking and unlocking the graphical session;
that path remains pending an instrumented test with the USB adapter left
connected.

The 13 analyzed frame samples consisted of one pre-existing frame with sequence
137 and the physical-reconnect run with sequences 2 through 13. They had only
three total transfer lengths: 85,040, 87,088, and 88,112 bytes. Every sample
obeyed the same structure:

```text
48-byte header | JPEG | 0x5000-byte CnM metadata area | zero padding
```

The header's little-endian 32-bit word at offset 16 was `0xa0000002`, bytes 20
and 21 were `0x04` and `0x01`, the sequence was in byte 22, and byte 23 stayed
zero. Values in this capture did not exceed `0x89`, so a wider sequence counter
is not yet established. The JPEG began with SOI at offset 48; walking its marker
structure located EOI, after which an exact `0x5000`-byte CnM area containing
non-zero data preceded the zero padding. Across all samples, total size was
`48 + align_up(jpeg_length + 0x5000, 1024)`.

Four header fields provisionally decode as a dirty rectangle: little-endian
16-bit `x`, `y`, `width`, and `height` at offsets 27, 29, 31, and 33. The
observed values were the full `1600x900` surface and aligned subrectangles
within it. The offsets and bounds are confirmed; their semantics remain an
inference until independently exercised.

Every captured frame had a one-to-one acknowledgement on interrupt IN
endpoint `0x81`. The response used opcode byte `0xe9` at offset 16 and repeated
the request's sequence byte at offset 20. Acknowledgements arrived 9--18 ms
after submission, with at most three frames awaiting acknowledgement. The USB
bulk completion itself arrived in about 0.52 ms and therefore is not evidence
that the device consumed a frame. The replacement must bound its in-flight
window and match acknowledgements by both connection generation and sequence.

Heartbeat requests (`0xec`) went out on bulk endpoint `0x02` at 1 Hz, while
`0xec` interrupt responses arrived continuously at 2 Hz. Offset 20 of those
responses tracked the most recently acknowledged frame in the two transitions
available in this capture; that interpretation is strong but still provisional.

The reconnect also exposed protocol and lifetime edge cases:

- USB address changed from 3 to 5, so physical port path plus a fresh
  generation, rather than address, must identify a session.
- The device reset once in place after re-enumeration. A session owner must
  cancel and drain work, then repeat configuration, claim, and alternate-setting
  setup even when the numerical address did not change.
- `-EINPROGRESS` is the normal usbmon status for submissions, `-ESHUTDOWN`
  marked detach completions, and `-ENOENT` appeared on cancelled HID endpoints;
  none should be treated as a generic transport crash.
- Header words 12 and 16 are opcode-specific. Captured opcode `0x45` used
  word 12 value 8 in a 44-byte transfer, while opcode `0x32` used word 16 value
  `0x10000006`. The strict length/class rules remain valid only for the
  canonical requests built by this project.

These are framing invariants from one SuperSpeed capture, not yet decoded
command semantics. The capture itself remains ignored and unpublished because
frame payloads can disclose screen contents; no payload or capture-specific
identifier is part of this repository.

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
