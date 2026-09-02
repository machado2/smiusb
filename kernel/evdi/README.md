# Patched EVDI kernel bridge

EVDI itself is GPL-2.0-only. This directory contains an open patch, not a copy
of Silicon Motion's installed source or any proprietary object.

The first patch closes the leading dma-buf vmap race identified in
[`docs/kernel-panic-2026-09-02.md`](../../docs/kernel-panic-2026-09-02.md). It
targets the exact SMI EVDI 1.14.16 source installed under `/usr/src` and refuses
unknown source trees. Complete 29-file manifests cover both the vendor baseline
and the patched source.

Build, install on disk, and register with DKMS:

```sh
./scripts/install-patched-evdi.sh
```

This does not stop the display service or unload the live module. It verifies
the DKMS source link and build artifact, refreshes the selected kernel's
initramfs, and proves that its embedded EVDI matches the patched module. The
patched module becomes active on the next normal reboot. To put the original
module back on disk and in every affected initramfs, without touching the live
monitor:

```sh
./scripts/rollback-patched-evdi.sh
```

The Rust userspace replacement is independent of this patch. This small kernel
bridge remains necessary while GNOME uses EVDI as the virtual KMS display.

## Validation

The patched tree builds through DKMS for kernels `7.0.0-29-generic` and
`7.0.0-30-generic`. It was also checked with Sparse `v0.6.5-rc1` at pinned
upstream commit `37156835e3d725b6d750f000be33ba3814bb2310`; its warnings are
identical to the unmodified SMI baseline.
