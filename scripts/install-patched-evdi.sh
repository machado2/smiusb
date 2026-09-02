#!/bin/sh
set -eu

PROJECT_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
SOURCE_DIR=${EVDI_SOURCE_DIR:-/usr/src/evdi-1.14.16}
PATCHED_VERSION=1.14.16.smiusb2
PATCHED_MODULE_VERSION=1.14.16-smiusb2
PATCHED_DIR=/usr/src/evdi-${PATCHED_VERSION}
PATCH_FILE=${PROJECT_ROOT}/kernel/evdi/patches/0001-serialize-persistent-vmap.patch
ORIGINAL_MANIFEST=${PROJECT_ROOT}/kernel/evdi/smi-1.14.16.sha256
PATCHED_MANIFEST=${PROJECT_ROOT}/kernel/evdi/smi-1.14.16.smiusb2.sha256
KERNEL_RELEASE=${1:-$(uname -r)}
TEMP_ROOT=

cleanup() {
	if [ -n "$TEMP_ROOT" ] && [ -d "$TEMP_ROOT" ]; then
		rm -rf -- "$TEMP_ROOT"
	fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

require_file() {
	if [ ! -f "$1" ]; then
		echo "missing required file: $1" >&2
		exit 1
	fi
}

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

	require_file "$initrd"
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
			echo "stale EVDI remains in $initrd: $module_path" >&2
			echo "expected version/srcversion/sha256: $expected_version/$expected_srcversion/$expected_sha256" >&2
			echo "actual version/srcversion/sha256: $actual_version/$actual_srcversion/$actual_sha256" >&2
			return 1
		fi
		index=$((index + 1))
	done
	echo "verified patched EVDI inside initramfs for $kernel_release"
}

case "$KERNEL_RELEASE" in
	*[!A-Za-z0-9._+-]*)
		echo "unsafe kernel release: $KERNEL_RELEASE" >&2
		exit 1
		;;
esac

require_file "$PATCH_FILE"
require_file "$ORIGINAL_MANIFEST"
require_file "$PATCHED_MANIFEST"
require_file "/lib/modules/$KERNEL_RELEASE/build/Makefile"

if ! validate_source_tree "$SOURCE_DIR" "$ORIGINAL_MANIFEST"; then
	echo "refusing to patch an unknown or non-canonical EVDI source tree: $SOURCE_DIR" >&2
	echo "this installer targets the locally shipped SMI EVDI 1.14.16 exactly" >&2
	exit 1
fi

TEMP_ROOT=$(mktemp -d -p /tmp smiusb-evdi.XXXXXX)

if [ ! -d "$PATCHED_DIR" ]; then
	STAGED_SOURCE=$TEMP_ROOT/evdi-$PATCHED_VERSION
	BUILD_SOURCE=$TEMP_ROOT/build-$PATCHED_VERSION
	cp -a --no-preserve=ownership -- "$SOURCE_DIR" "$STAGED_SOURCE"
	patch --directory="$STAGED_SOURCE" --strip=1 --forward --input="$PATCH_FILE"
	if ! validate_source_tree "$STAGED_SOURCE" "$PATCHED_MANIFEST"; then
		echo "patched staging tree failed complete manifest validation" >&2
		exit 1
	fi
	cp -a -- "$STAGED_SOURCE" "$BUILD_SOURCE"
	make -C "$BUILD_SOURCE" -j"$(getconf _NPROCESSORS_ONLN)" KVER="$KERNEL_RELEASE"
	if [ "$(modinfo -F version "$BUILD_SOURCE/evdi.ko")" != "$PATCHED_MODULE_VERSION" ]; then
		echo "staged module did not pass version verification" >&2
		exit 1
	fi
	# DKMS compiles privileged kernel code from /usr/src. Do not preserve the
	# unprivileged staging owner when installing that source tree.
	sudo cp -a --no-preserve=ownership -- "$STAGED_SOURCE" "$PATCHED_DIR"
	sudo chown -R root:root -- "$PATCHED_DIR"
	sudo chmod -R go-w -- "$PATCHED_DIR"
fi

# Never repair an existing unsafe tree in place: an already-open descriptor
# would remain writable after chown. Reject it before any privileged build.
UNSAFE_PATH=$(sudo find "$PATCHED_DIR" -xdev \
	\( ! -user root -o ! -group root -o -perm /022 \) -print -quit)
if [ -n "$UNSAFE_PATH" ]; then
	echo "patched DKMS source has unsafe ownership or permissions" >&2
	exit 1
fi
if ! validate_source_tree "$PATCHED_DIR" "$PATCHED_MANIFEST"; then
	echo "existing patched source failed complete validation: $PATCHED_DIR" >&2
	exit 1
fi

CURRENT_MODULE=$(modinfo -k "$KERNEL_RELEASE" -n evdi 2>/dev/null || true)
CURRENT_VERSION=
if [ -n "$CURRENT_MODULE" ] && [ -f "$CURRENT_MODULE" ]; then
	CURRENT_VERSION=$(modinfo -F version "$CURRENT_MODULE" 2>/dev/null || true)
fi
if [ -n "$CURRENT_MODULE" ] && [ -f "$CURRENT_MODULE" ] &&
	[ "$CURRENT_VERSION" != "$PATCHED_MODULE_VERSION" ]; then
	BACKUP_DIR=/var/lib/smiusb/evdi-backup/$KERNEL_RELEASE
	if [ ! -e "$BACKUP_DIR/$(basename -- "$CURRENT_MODULE")" ]; then
		sudo install -d -m 0755 "$BACKUP_DIR"
		sudo install -m 0644 "$CURRENT_MODULE" "$BACKUP_DIR/$(basename -- "$CURRENT_MODULE")"
	fi
fi

if ! dkms status -m evdi -v "$PATCHED_VERSION" 2>/dev/null | grep -q .; then
	sudo dkms add -m evdi -v "$PATCHED_VERSION"
fi
verify_dkms_source "$PATCHED_VERSION" "$PATCHED_DIR"
sudo dkms build -m evdi -v "$PATCHED_VERSION" -k "$KERNEL_RELEASE" --force
BUILT_MODULE=$(find_dkms_module "$PATCHED_VERSION" "$KERNEL_RELEASE")
BUILT_RAW=${TEMP_ROOT}/built-evdi.ko
decompress_module "$BUILT_MODULE" "$BUILT_RAW"
BUILT_VERSION=$(modinfo -F version "$BUILT_RAW" 2>/dev/null || true)
BUILT_SRCVERSION=$(modinfo -F srcversion "$BUILT_RAW" 2>/dev/null || true)
BUILT_VERMAGIC=$(modinfo -F vermagic "$BUILT_RAW" 2>/dev/null || true)
BUILT_SHA256=$(sha256sum "$BUILT_RAW" | cut -d ' ' -f 1)
case "$BUILT_VERMAGIC" in
	"$KERNEL_RELEASE "*) ;;
	*)
		echo "DKMS artifact vermagic does not target $KERNEL_RELEASE" >&2
		exit 1
		;;
esac
if [ "$BUILT_VERSION" != "$PATCHED_MODULE_VERSION" ] ||
	[ -z "$BUILT_SRCVERSION" ]; then
	echo "DKMS build artifact failed identity verification: $BUILT_MODULE" >&2
	exit 1
fi
sudo dkms install -m evdi -v "$PATCHED_VERSION" -k "$KERNEL_RELEASE" --force

ON_DISK_MODULE=$(modinfo -k "$KERNEL_RELEASE" -n evdi)
ON_DISK_RAW=${TEMP_ROOT}/on-disk-evdi.ko
decompress_module "$ON_DISK_MODULE" "$ON_DISK_RAW"
ON_DISK_VERSION=$(modinfo -F version "$ON_DISK_RAW" 2>/dev/null || true)
ON_DISK_SRCVERSION=$(modinfo -F srcversion "$ON_DISK_RAW" 2>/dev/null || true)
ON_DISK_VERMAGIC=$(modinfo -F vermagic "$ON_DISK_RAW" 2>/dev/null || true)
ON_DISK_SHA256=$(sha256sum "$ON_DISK_RAW" | cut -d ' ' -f 1)
if [ "$ON_DISK_VERSION" != "$BUILT_VERSION" ] ||
	[ "$ON_DISK_SRCVERSION" != "$BUILT_SRCVERSION" ] ||
	[ "$ON_DISK_VERMAGIC" != "$BUILT_VERMAGIC" ] ||
	[ "$ON_DISK_SHA256" != "$BUILT_SHA256" ]; then
	echo "installed module did not pass version verification: $ON_DISK_MODULE" >&2
	exit 1
fi

# Dracut copied the vendor module into this host's initramfs. Refresh it and
# verify the embedded object by version identity before reboot.
sudo update-initramfs -u -k "$KERNEL_RELEASE"
verify_initramfs_evdi "$KERNEL_RELEASE" "$BUILT_VERSION" "$BUILT_SRCVERSION" \
	"$BUILT_VERMAGIC" "$BUILT_SHA256"

echo "installed patched EVDI for $KERNEL_RELEASE: $ON_DISK_MODULE"
if [ -d /sys/module/evdi ]; then
	echo "the currently loaded module and monitor were left untouched"
	echo "the patched module will become active on the next normal reboot"
fi
