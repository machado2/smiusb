#!/bin/sh
set -eu

PROJECT_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
PATCHED_VERSION=1.14.16.smiusb2
LEGACY_PATCHED_VERSION=1.14.16.smiusb1
ORIGINAL_VERSION=1.14.16
ORIGINAL_SOURCE=/usr/src/evdi-$ORIGINAL_VERSION
ORIGINAL_MANIFEST=${PROJECT_ROOT}/kernel/evdi/smi-1.14.16.sha256
TEMP_ROOT=

cleanup() {
	if [ -n "$TEMP_ROOT" ] && [ -d "$TEMP_ROOT" ]; then
		rm -rf -- "$TEMP_ROOT"
	fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

validate_source_tree() {
	tree=$1
	manifest=$2
	expected_count=$(wc -l < "$manifest")
	actual_count=$(find "$tree" -mindepth 1 -maxdepth 1 -type f -print | wc -l)

	[ "$actual_count" -eq "$expected_count" ] || return 1
	[ -z "$(find "$tree" -mindepth 1 -maxdepth 1 ! -type f -print -quit)" ] || return 1
	(CDPATH='' cd -- "$tree" && sha256sum --check --strict --status "$manifest")
}

decompress_module() {
	compressed_module=$1
	raw_module=$2
	case "$compressed_module" in
		*.ko.zstd) zstd --quiet --decompress --stdout "$compressed_module" > "$raw_module" ;;
		*.ko.zst) zstd --quiet --decompress --stdout "$compressed_module" > "$raw_module" ;;
		*.ko.xz) xz --decompress --stdout "$compressed_module" > "$raw_module" ;;
		*.ko.gz) gzip --decompress --stdout "$compressed_module" > "$raw_module" ;;
		*.ko.bz2) bzip2 --decompress --stdout "$compressed_module" > "$raw_module" ;;
		*.ko.lzma) lzma --decompress --stdout "$compressed_module" > "$raw_module" ;;
		*.ko.lz4) lz4 --quiet --decompress --stdout "$compressed_module" > "$raw_module" ;;
		*.ko) cp -- "$compressed_module" "$raw_module" ;;
		*)
			echo "unsupported kernel module compression: $compressed_module" >&2
			return 1
			;;
	esac
}

verify_dkms_source() {
	dkms_version=$1
	expected_source=$2
	dkms_link=/var/lib/dkms/evdi/${dkms_version}/source
	actual_source=$(readlink -f "$dkms_link" 2>/dev/null || true)
	expected_source=$(readlink -f "$expected_source" 2>/dev/null || true)
	if [ -z "$actual_source" ] || [ "$actual_source" != "$expected_source" ]; then
		echo "DKMS source mismatch for evdi/$dkms_version: $actual_source" >&2
		return 1
	fi
}

find_dkms_module() {
	dkms_version=$1
	kernel_release=$2
	dkms_build_root=/var/lib/dkms/evdi/${dkms_version}/${kernel_release}
	modules=$(find "$dkms_build_root" -type f -path '*/module/evdi.ko*' -print 2>/dev/null || true)
	module_count=$(printf '%s\n' "$modules" | awk 'NF { count++ } END { print count + 0 }')
	if [ "$module_count" -ne 1 ]; then
		echo "expected one DKMS evdi artifact for $kernel_release, found $module_count" >&2
		return 1
	fi
	printf '%s\n' "$modules"
}

verify_initramfs_evdi() {
	kernel_release=$1
	expected_version=$2
	expected_srcversion=$3
	expected_vermagic=$4
	expected_sha256=$5
	initrd=/boot/initrd.img-${kernel_release}
	initrd_listing=${TEMP_ROOT}/initrd-${kernel_release}.list

	if [ ! -f "$initrd" ]; then
		echo "missing initramfs: $initrd" >&2
		return 1
	fi
	# The destination is intentionally owned by the invoking user in TEMP_ROOT.
	# shellcheck disable=SC2024
	if ! sudo lsinitramfs "$initrd" > "$initrd_listing"; then
		echo "failed to inspect initramfs: $initrd" >&2
		return 1
	fi
	module_paths=$(awk -v usr_prefix="usr/lib/modules/${kernel_release}/" \
		-v lib_prefix="lib/modules/${kernel_release}/" '
		{
			path = $0
			sub(/^\.[/]/, "", path)
			if ((index(path, usr_prefix) == 1 || index(path, lib_prefix) == 1) &&
			    path ~ /\/evdi[.]ko[^/]*$/)
				print $0
		}' "$initrd_listing")
	if [ -z "$module_paths" ]; then
		echo "verified initramfs for $kernel_release: no embedded EVDI copy"
		return 0
	fi

	index=0
	for module_path in $module_paths; do
		case "$module_path" in
			*.ko.zstd) suffix=.ko.zstd ;;
			*.ko.zst) suffix=.ko.zst ;;
			*.ko.xz) suffix=.ko.xz ;;
			*.ko.gz) suffix=.ko.gz ;;
			*.ko.bz2) suffix=.ko.bz2 ;;
			*.ko.lzma) suffix=.ko.lzma ;;
			*.ko.lz4) suffix=.ko.lz4 ;;
			*.ko) suffix=.ko ;;
			*)
				echo "unsupported initramfs module name: $module_path" >&2
				return 1
				;;
		esac
		audit_module=${TEMP_ROOT}/initrd-evdi-${index}${suffix}
		audit_raw=${TEMP_ROOT}/initrd-evdi-${index}.ko
		# Redirection intentionally writes into the unprivileged temporary tree.
		# shellcheck disable=SC2024
		sudo lsinitrd -f "$module_path" "$initrd" > "$audit_module"
		decompress_module "$audit_module" "$audit_raw"
		actual_version=$(modinfo -F version "$audit_raw" 2>/dev/null || true)
		actual_srcversion=$(modinfo -F srcversion "$audit_raw" 2>/dev/null || true)
		actual_vermagic=$(modinfo -F vermagic "$audit_raw" 2>/dev/null || true)
		actual_sha256=$(sha256sum "$audit_raw" | cut -d ' ' -f 1)
		if [ "$actual_version" != "$expected_version" ] ||
			[ "$actual_srcversion" != "$expected_srcversion" ] ||
			[ "$actual_vermagic" != "$expected_vermagic" ] ||
			[ "$actual_sha256" != "$expected_sha256" ]; then
			echo "unexpected EVDI remains in $initrd: $module_path" >&2
			return 1
		fi
		index=$((index + 1))
	done
}

if [ "$#" -ne 0 ]; then
	echo "usage: $0" >&2
	echo "rollback is failure-safe and ordered across every patched kernel" >&2
	exit 2
fi

if [ ! -d "$ORIGINAL_SOURCE" ]; then
	echo "original EVDI source is missing: $ORIGINAL_SOURCE" >&2
	exit 1
fi
UNSAFE_ORIGINAL=$(sudo find "$ORIGINAL_SOURCE" -xdev \
	\( ! -user root -o ! -group root -o -perm /022 \) -print -quit)
if [ -n "$UNSAFE_ORIGINAL" ] || [ ! -f "$ORIGINAL_MANIFEST" ] ||
	! validate_source_tree "$ORIGINAL_SOURCE" "$ORIGINAL_MANIFEST"; then
	echo "original EVDI source failed ownership or integrity validation" >&2
	exit 1
fi

PATCHED_KERNELS=$(
	{
		dkms status -m evdi -v "$PATCHED_VERSION" 2>/dev/null || true
		dkms status -m evdi -v "$LEGACY_PATCHED_VERSION" 2>/dev/null || true
	} | awk -F ', ' '$3 ~ /: (built|installed)([[:space:]]|$)/ { print $2 }' | sort -u
)

if [ -z "$PATCHED_KERNELS" ]; then
	for version in "$PATCHED_VERSION" "$LEGACY_PATCHED_VERSION"; do
		if dkms status -m evdi -v "$version" 2>/dev/null | grep -q .; then
			sudo dkms remove -m evdi -v "$version" --all
		fi
	done
	echo "no kernel had patched EVDI selected; removed its DKMS registration"
	exit 0
fi

TEMP_ROOT=$(mktemp -d -p /tmp smiusb-evdi-rollback.XXXXXX)

if ! dkms status -m evdi -v "$ORIGINAL_VERSION" 2>/dev/null | grep -q .; then
	sudo dkms add -m evdi -v "$ORIGINAL_VERSION"
fi
verify_dkms_source "$ORIGINAL_VERSION" "$ORIGINAL_SOURCE"

# Complete every build before changing the selected module for any kernel.
for KERNEL_RELEASE in $PATCHED_KERNELS; do
	case "$KERNEL_RELEASE" in
		*[!A-Za-z0-9._+-]*)
			echo "unsafe kernel release from DKMS: $KERNEL_RELEASE" >&2
			exit 1
			;;
	esac
	if [ ! -f "/lib/modules/$KERNEL_RELEASE/build/Makefile" ]; then
		echo "kernel headers are missing for $KERNEL_RELEASE" >&2
		exit 1
	fi
	sudo dkms build -m evdi -v "$ORIGINAL_VERSION" -k "$KERNEL_RELEASE" --force
done

# Validate every build artifact before changing the selected module for the
# first kernel. This keeps failures in the preflight phase whenever possible.
for KERNEL_RELEASE in $PATCHED_KERNELS; do
	BUILT_MODULE=$(find_dkms_module "$ORIGINAL_VERSION" "$KERNEL_RELEASE")
	BUILT_RAW=${TEMP_ROOT}/preflight-${KERNEL_RELEASE}.ko
	decompress_module "$BUILT_MODULE" "$BUILT_RAW"
	BUILT_SRCVERSION=$(modinfo -F srcversion "$BUILT_RAW" 2>/dev/null || true)
	BUILT_VERMAGIC=$(modinfo -F vermagic "$BUILT_RAW" 2>/dev/null || true)
	case "$BUILT_VERMAGIC" in
		"$KERNEL_RELEASE "*) ;;
		*)
			echo "original DKMS artifact vermagic does not target $KERNEL_RELEASE" >&2
			exit 1
			;;
	esac
	if [ -z "$BUILT_SRCVERSION" ]; then
		echo "original DKMS artifact has no srcversion for $KERNEL_RELEASE" >&2
		exit 1
	fi
done

# Select, verify, and put the original into every boot image before deleting
# the patched registration. Every intermediate kernel remains bootable.
for KERNEL_RELEASE in $PATCHED_KERNELS; do
	BUILT_MODULE=$(find_dkms_module "$ORIGINAL_VERSION" "$KERNEL_RELEASE")
	BUILT_RAW=${TEMP_ROOT}/built-${KERNEL_RELEASE}.ko
	decompress_module "$BUILT_MODULE" "$BUILT_RAW"
	BUILT_VERSION=$(modinfo -F version "$BUILT_RAW" 2>/dev/null || true)
	BUILT_SRCVERSION=$(modinfo -F srcversion "$BUILT_RAW" 2>/dev/null || true)
	BUILT_VERMAGIC=$(modinfo -F vermagic "$BUILT_RAW" 2>/dev/null || true)
	BUILT_SHA256=$(sha256sum "$BUILT_RAW" | cut -d ' ' -f 1)
	if [ -z "$BUILT_SRCVERSION" ] || [ -z "$BUILT_VERMAGIC" ]; then
		echo "original DKMS artifact failed identity verification for $KERNEL_RELEASE" >&2
		exit 1
	fi
	sudo dkms install -m evdi -v "$ORIGINAL_VERSION" -k "$KERNEL_RELEASE" --force
	ORIGINAL_MODULE=$(modinfo -k "$KERNEL_RELEASE" -n evdi)
	ORIGINAL_RAW=${TEMP_ROOT}/on-disk-${KERNEL_RELEASE}.ko
	decompress_module "$ORIGINAL_MODULE" "$ORIGINAL_RAW"
	ORIGINAL_MODULE_VERSION=$(modinfo -F version "$ORIGINAL_RAW" 2>/dev/null || true)
	ORIGINAL_SRCVERSION=$(modinfo -F srcversion "$ORIGINAL_RAW" 2>/dev/null || true)
	ORIGINAL_VERMAGIC=$(modinfo -F vermagic "$ORIGINAL_RAW" 2>/dev/null || true)
	ORIGINAL_SHA256=$(sha256sum "$ORIGINAL_RAW" | cut -d ' ' -f 1)
	if ! dkms status -m evdi -v "$ORIGINAL_VERSION" -k "$KERNEL_RELEASE" 2>/dev/null | grep -q installed ||
		[ "$ORIGINAL_MODULE_VERSION" != "$BUILT_VERSION" ] ||
		[ "$ORIGINAL_SRCVERSION" != "$BUILT_SRCVERSION" ] ||
		[ "$ORIGINAL_VERMAGIC" != "$BUILT_VERMAGIC" ] ||
		[ "$ORIGINAL_SHA256" != "$BUILT_SHA256" ]; then
		echo "original EVDI failed post-install verification for $KERNEL_RELEASE" >&2
		echo "patched DKMS registration was retained for recovery" >&2
		exit 1
	fi
	sudo update-initramfs -u -k "$KERNEL_RELEASE"
	verify_initramfs_evdi "$KERNEL_RELEASE" "$BUILT_VERSION" "$BUILT_SRCVERSION" \
		"$BUILT_VERMAGIC" "$BUILT_SHA256"
done

for version in "$PATCHED_VERSION" "$LEGACY_PATCHED_VERSION"; do
	if dkms status -m evdi -v "$version" 2>/dev/null | grep -q .; then
		sudo dkms remove -m evdi -v "$version" --all
	fi
done

for KERNEL_RELEASE in $PATCHED_KERNELS; do
	if ! dkms status -m evdi -v "$ORIGINAL_VERSION" -k "$KERNEL_RELEASE" 2>/dev/null | grep -q installed; then
		echo "rollback verification failed after DKMS cleanup for $KERNEL_RELEASE" >&2
		exit 1
	fi
	ORIGINAL_MODULE=$(modinfo -k "$KERNEL_RELEASE" -n evdi)
	ORIGINAL_RAW=${TEMP_ROOT}/final-${KERNEL_RELEASE}.ko
	decompress_module "$ORIGINAL_MODULE" "$ORIGINAL_RAW"
	ORIGINAL_MODULE_VERSION=$(modinfo -F version "$ORIGINAL_RAW" 2>/dev/null || true)
	ORIGINAL_SRCVERSION=$(modinfo -F srcversion "$ORIGINAL_RAW" 2>/dev/null || true)
	ORIGINAL_VERMAGIC=$(modinfo -F vermagic "$ORIGINAL_RAW" 2>/dev/null || true)
	ORIGINAL_SHA256=$(sha256sum "$ORIGINAL_RAW" | cut -d ' ' -f 1)
	BUILT_MODULE=$(find_dkms_module "$ORIGINAL_VERSION" "$KERNEL_RELEASE")
	BUILT_RAW=${TEMP_ROOT}/final-built-${KERNEL_RELEASE}.ko
	decompress_module "$BUILT_MODULE" "$BUILT_RAW"
	BUILT_VERSION=$(modinfo -F version "$BUILT_RAW" 2>/dev/null || true)
	BUILT_SRCVERSION=$(modinfo -F srcversion "$BUILT_RAW" 2>/dev/null || true)
	BUILT_VERMAGIC=$(modinfo -F vermagic "$BUILT_RAW" 2>/dev/null || true)
	BUILT_SHA256=$(sha256sum "$BUILT_RAW" | cut -d ' ' -f 1)
	if [ "$ORIGINAL_MODULE_VERSION" != "$BUILT_VERSION" ] ||
		[ "$ORIGINAL_SRCVERSION" != "$BUILT_SRCVERSION" ] ||
		[ "$ORIGINAL_VERMAGIC" != "$BUILT_VERMAGIC" ] ||
		[ "$ORIGINAL_SHA256" != "$BUILT_SHA256" ]; then
		echo "original EVDI identity is missing for $KERNEL_RELEASE" >&2
		exit 1
	fi
	verify_initramfs_evdi "$KERNEL_RELEASE" "$BUILT_VERSION" "$BUILT_SRCVERSION" \
		"$BUILT_VERMAGIC" "$BUILT_SHA256"
done

echo "restored original EVDI $ORIGINAL_VERSION for:"
for KERNEL_RELEASE in $PATCHED_KERNELS; do
	printf '  %s\n' "$KERNEL_RELEASE"
done
if [ -d /sys/module/evdi ]; then
	echo "the currently loaded module and monitor were left untouched"
	echo "the rollback will become active on the next normal reboot"
fi
