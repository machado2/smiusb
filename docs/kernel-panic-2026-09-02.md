# EVDI dma-buf panic, 2026-09-02

This incident is separate from the vendor daemon's previously confirmed USB
disconnect use-after-free. It is a kernel-side dma-buf lifetime invariant
failure that occurred immediately after EVDI enabled the USB display. The
available dump proves the invariant failure, but not which component or call
sequence caused the outstanding reference.

## Preserved evidence

- Host kernel: Ubuntu `7.0.0-30-generic`.
- Loaded EVDI source version: Silicon Motion's modified `1.14.16` tree.
- Compositor: GNOME Shell on Wayland, with i915 and EVDI active. A later live
  sample showed an i915-exported buffer attached to EVDI, but the crash log
  alone cannot identify the exporter of the failed object.
- Crash log: `/var/crash/202609021946/dmesg.202609021946` (kept outside this
  repository), 280,852 bytes, SHA-256
  `e1eb4f7cc447099edbc8ce5447928e73867f6da1f8fb26d619117be029b730b6`.
- No `vmcore` was produced; `dump-incomplete` is empty. Conclusions that would
  require inspecting heap objects in a vmcore remain hypotheses.

The capture kernel was forcibly interrupted about 52 seconds into the full
dump. `makedumpfile` scans memory to build its bitmap before emitting the
flattened header, so a zero-byte `dump-incomplete` is consistent with that
interruption. Its Linux-version warning was not the direct failure: the smaller
dmesg extraction printed the same warning and completed successfully.

At kernel time 43787.114750 EVDI announced card2 at 1600x900@60. At
43787.217033, 102.283 ms later, GNOME Shell hit:

```text
kernel BUG at drivers/dma-buf/dma-buf.c:174
RIP: dma_buf_release+0x90/0xa0
RDX: 00007ffddff64aa0 RSI: 0000000040086409 RDI: 0000000000000025
```

Request `0x40086409` is `DRM_IOCTL_GEM_CLOSE`. In Linux v7.0, line 174 is
exactly `BUG_ON(dmabuf->vmapping_counter)`. Disassembly of the matching Ubuntu
kernel image confirms that the branch to the `UD2` at `dma_buf_release+0x90`
tests `dma_buf.vmapping_counter`. Register `RCX` held `1`, proving one
outstanding vmap reference at release. That is consistent with a missing
vunmap, but premature final release or counter corruption are also possible.

## Trigger correlation

The panic was not caused by a physical USB detach/reconnect. The final ten
minutes preserved by the capture kernel contain no USB disconnect, new-device,
reset, or EVDI disconnect/connect event. The persistent journal shows that the
SM768 had last disconnected and re-enumerated more than eight hours earlier,
at 11:13:14--11:13:15, and remained present through the panic. There was also
no system suspend/resume transition near the failure.

The evidence instead supports a graphical-session wake path. At 15:52:26,
GNOME Shell logged an AccountsService password-mode lookup that the installed
GNOME Shell 50.1 performs in its screen-shield lock method. This suggests lock
path activity rather than recording a lock signal directly. The lid closed one
second later; EVDI then went off, briefly back on with a mode, and finally off
again at 15:52:58. No later EVDI transition is preserved before the 19:46:12
power-on and mode notification immediately preceding the BUG. The failing
GNOME Shell system call was `DRM_IOCTL_GEM_CLOSE`, so a compositor GEM handle
was being released as EVDI reactivated its CRTC. Together with the user's
recollection of locking and unlocking the machine, session unlock/DPMS wake is
the most likely trigger. GNOME emitted no explicit unlock record before the
panic, so the exact user action remains strongly supported rather than proven
solely by the logs.

Later manual USB detach/reconnect cycles were deliberate tests, not the panic
trigger. The first post-patch cycle at 21:38:16 completed without a new BUG,
Oops, panic, or userspace coredump; it validates only the physical-detach guard
path and does not yet validate lock/unlock.

Passive tracing on the next boot demonstrated the candidate ownership chain:
an SMI userspace update thread entered an EVDI path that vmap'ed i915 buffers,
and an EVDI free path ran in GNOME Shell context. Three ordinary samples were
balanced. This establishes that the path exists, not that the crashed dma-buf
followed it or that the proposed race occurred.

## Candidate race in EVDI

`evdi_painter_grabpix_ioctl()` checks `obj->vmapping` after dropping the painter
lock, then calls `evdi_gem_vmap()`. The GEM object has a pages mutex, but its
persistent `vmapping` field has no lock. Two concurrent GRABPIX callers, or a
GRABPIX and cursor caller, can therefore both observe NULL and both call
`dma_buf_vmap_unlocked()`:

```text
worker A: counter 0 -> 1, stores mapping
worker B: counter 1 -> 2, stores the same mapping
GEM free: one vunmap, counter 2 -> 1
dma-buf final close: BUG_ON(counter != 0)
```

This permitted race predicts exactly the observed remainder of one, making it
the strongest actionable mechanism found. It is still a hypothesis: without
the vmcore, the failed dma-buf cannot be tied to an EVDI GEM object, and an
early final release, another missing unmap, or counter corruption remain
possible.

EVDI v1.15.0 and current upstream still access the same field without
serialization, so a version-only upgrade would not remove this candidate.

## Open patch

[`kernel/evdi/patches/0001-serialize-persistent-vmap.patch`](../kernel/evdi/patches/0001-serialize-persistent-vmap.patch)
adds a per-object mutex, sends both mapping call sites through it, rechecks the
persistent mapping while holding it, and makes unmap idempotent. It also
balances `pages_pin_count` when a local vmap allocation fails. This closes the
concrete race above without claiming that the missing vmcore can establish
unique causality. The patch changes no EVDI userspace ABI.

The patch intentionally targets the exact SMI-modified 1.14.16 source already
installed on this host. Replacing that source wholesale with upstream 1.15.0
would also remove SMI changes to cursor, EDID, VT, and AMD behavior and is a
larger regression risk. The installer checks complete source manifests, builds
through DKMS, refreshes and verifies the initramfs copy, and never unloads the
live module. The monitor remains active; the new module takes effect only at
the next normal reboot.

## Primary references

- [Linux v7.0 dma_buf_release invariant](https://github.com/torvalds/linux/blob/v7.0/drivers/dma-buf/dma-buf.c#L166-L181)
- [Linux v7.0 vmap counter implementation](https://github.com/torvalds/linux/blob/v7.0/drivers/dma-buf/dma-buf.c#L1479-L1591)
- [Upstream EVDI v1.14.16 mapping baseline](https://github.com/DisplayLink/evdi/blob/v1.14.16/module/evdi_gem.c#L369-L468)
- [EVDI v1.15.0 release](https://github.com/DisplayLink/evdi/releases/tag/v1.15.0)
