# Open SM768 driver architecture

The replacement is split at the same boundary as modern USB display stacks,
but every component is open:

```text
GNOME RecordVirtual -> PipeWire BGRx -> Rust bounded queue
                    -> TurboJPEG 4:2:2 -> SM768 packetizer
                    -> one Rust libusb session owner -> interface 4
```

During development, the existing service continues to drive the monitor. The
open daemon does not claim interface 4 or transmit provisional packets until a
complete attach and frame trace has been decoded. This lets the replacement be
built without taking away the user's display.

## Components

### Virtual monitor and capture

GNOME Mutter's ScreenCast API v4 is present on the development host. The
planned production path uses `RecordVirtual` to create an actual desktop
monitor and receives its frames over PipeWire. BGRx is the preferred negotiated
format because it matches the observed vendor TurboJPEG input and avoids a
color conversion. VKMS remains a non-GNOME fallback, not the primary path.

The Rust prototype already validates BGRx frame sizes and implements a bounded
drop-old queue. Every frame has `(generation, sequence)` identity. Reconnect
increments the generation, atomically discards queued work from the old USB
device, and rejects late frames. Encoded frames retain that identity, and a
generation gate rejects stale work immediately before the future USB sink.

### Encoder and SM768 framing

The observed encoder uses JPEG 4:2:2. Offline deterministic color bars and a
moving square provide known input for encoder and packetizer tests without
accessing private screen contents. Frame and JPEG allocations are bounded and
fallible; the TurboJPEG scratch buffer is reused by its single owner.

The outer frame layout is confirmed by 13 samples from a real SuperSpeed
physical USB reconnect capture:

```text
48-byte header | JPEG bytes | 0x5000-byte CnM area | zero padding
```

The CnM area contains structured non-zero data but is exactly `0x5000` bytes;
it is not padding. The JPEG SOI is at offset 48 and its marker structure
determines EOI. For every sample, total transfer length was
`48 + align_up(jpeg_length + 0x5000, 1024)`. The earlier internal-allocation
inference of an extra byte at a 512-byte boundary is not a wire-format rule.
The observed sequence byte is at header offset 22, while offset 23 stayed zero;
the capture does not yet establish whether that unclassified byte extends the
counter. The frames also consistently had `0xa0000002` at offset 16 and bytes
`0x04 0x01` at offsets 20 and 21. The semantics of those fixed fields and the
non-zero CnM area remain to be decoded. USB transmission stays disabled until
the offline parser/replayer validates them together with mode commands and
acknowledgements.

### USB ownership

The production transport will give one worker sole ownership of a libusb
handle. Its event callback will enqueue a referenced device but never close it
or start detached work. A disconnect will perform this ordered transition:

1. increment the connection generation and reject old work;
2. cancel outstanding asynchronous transfers;
3. drain their callbacks;
4. join every owned worker;
5. release only interface 4;
6. close the handle on its owner thread.

HID and audio interfaces remain attached to their normal kernel drivers. Bulk
max-packet size is read from the active alternate setting because this device
reports 512 bytes at High Speed and 1024 at SuperSpeed.

The first reconnect trace also establishes that a numerical USB address is not
session identity: the adapter moved from address 3 to 5 on the same physical
path and performed an additional reset after re-enumeration. Either event must
advance the generation and repeat configuration, claim, and alternate-setting
setup. Frame bulk completions occur long before semantic `0xe9`
acknowledgements; the transport therefore needs a bounded in-flight window
(three frames in the captured burst) keyed by generation and sequence byte.

## Delivery gates

1. **Done:** passive Rust enumeration, strict response decoder, deterministic
   request builders, generation-safe frame queue, test-pattern producer.
2. **Done:** bounded BGRx-to-JPEG 4:2:2 encoding through TurboJPEG, including
   deterministic roundtrip and memory-safety tests.
3. **In progress:** PipeWire frame capture through Mutter ScreenCast v4.
4. **Partially done:** post-reboot kernel `7.0.0-30` loaded EVDI
   `1.14.16-smiusb2`; one manually induced physical USB detach/reconnect
   reattached in about 6.91 seconds, restored `1600x900@60`, and captured real
   frame traffic with no BUG, Oops, panic, or coredump. This validates only the
   physical-detach path, not the initially suspected KVM path or the now
   suspected session lock/unlock trigger. The full attach/EDID/mode-set exchange
   still needs to be decoded, and lock/unlock remains pending an instrumented
   test with the adapter connected.
5. **In progress:** the offline Rust parser now validates the captured frame
   header, structural JPEG EOI, exact `0x5000` metadata region, zero padding,
   512/1024-byte alignment, and strict `0xe9` frame acknowledgements. Decoding
   the CnM contents and remaining attach commands, fuzzing, and offline replay
   still precede any USB OUT path.
6. Add an explicit experimental transport flag and validate on the local
   adapter while the vendor service can be restored immediately.
7. Only after repeated physical USB detach/reconnect, session lock/unlock, and
   suspend/resume tests, install the Rust service in place of the proprietary
   daemon.
