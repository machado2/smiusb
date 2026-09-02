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

The provisional frame layout is:

```text
48-byte transport header | JPEG bytes | 0x5000-byte CnM decoder area
```

Total allocation is `jpeg_length + 0x5030`, plus one byte when that value is an
exact multiple of 512. This layout comes from the vendor packet builder. USB
transmission stays disabled until a real frame capture confirms it and reveals
the header fields, decoder metadata, mode command, and acknowledgement rules.

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

## Delivery gates

1. **Done:** passive Rust enumeration, strict response decoder, deterministic
   request builders, generation-safe frame queue, test-pattern producer.
2. **Done:** bounded BGRx-to-JPEG 4:2:2 encoding through TurboJPEG, including
   deterministic roundtrip and memory-safety tests.
3. **In progress:** PipeWire frame capture through Mutter ScreenCast v4.
4. Capture a complete attach, EDID/mode-set exchange, and a real test-pattern
   frame after the patched EVDI has survived reconnect stress.
5. Implement and fuzz an offline packet decoder/replayer before enabling the
   USB OUT path.
6. Add an explicit experimental transport flag and validate on the local
   adapter while the vendor service can be restored immediately.
7. Only after repeated KVM and suspend tests, install the Rust service in place
   of the proprietary daemon.
