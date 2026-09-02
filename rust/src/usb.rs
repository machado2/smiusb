use crate::protocol::{SM768_PRODUCT_ID, VENDOR_ID};
use std::collections::BTreeMap;
use std::ffi::{CStr, c_char, c_int, c_uchar};
use std::marker::PhantomData;
use std::ptr;

#[repr(C)]
struct LibusbContext {
    _private: [u8; 0],
}

#[repr(C)]
struct LibusbDevice {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LibusbDeviceDescriptor {
    b_length: u8,
    b_descriptor_type: u8,
    bcd_usb: u16,
    b_device_class: u8,
    b_device_sub_class: u8,
    b_device_protocol: u8,
    b_max_packet_size0: u8,
    id_vendor: u16,
    id_product: u16,
    bcd_device: u16,
    i_manufacturer: u8,
    i_product: u8,
    i_serial_number: u8,
    b_num_configurations: u8,
}

#[link(name = "usb-1.0")]
unsafe extern "C" {
    fn libusb_init(context: *mut *mut LibusbContext) -> c_int;
    fn libusb_exit(context: *mut LibusbContext);
    fn libusb_get_device_list(
        context: *mut LibusbContext,
        list: *mut *mut *mut LibusbDevice,
    ) -> isize;
    fn libusb_free_device_list(list: *mut *mut LibusbDevice, unref_devices: c_int);
    fn libusb_get_device_descriptor(
        device: *mut LibusbDevice,
        descriptor: *mut LibusbDeviceDescriptor,
    ) -> c_int;
    fn libusb_get_bus_number(device: *mut LibusbDevice) -> c_uchar;
    fn libusb_get_device_address(device: *mut LibusbDevice) -> c_uchar;
    fn libusb_get_port_number(device: *mut LibusbDevice) -> c_uchar;
    fn libusb_get_device_speed(device: *mut LibusbDevice) -> c_int;
    fn libusb_error_name(error_code: c_int) -> *const c_char;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DeviceKey {
    pub bus: u8,
    pub address: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceInfo {
    pub key: DeviceKey,
    pub port: u8,
    pub usb_version: u16,
    pub device_version: u16,
    pub speed: Speed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Speed {
    Unknown,
    Low,
    Full,
    High,
    Super,
    SuperPlus,
    Other(i32),
}

impl From<i32> for Speed {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Unknown,
            1 => Self::Low,
            2 => Self::Full,
            3 => Self::High,
            4 => Self::Super,
            5 => Self::SuperPlus,
            other => Self::Other(other),
        }
    }
}

pub struct UsbContext {
    raw: *mut LibusbContext,
    // libusb contexts are deliberately confined to the creating thread.
    _not_send_or_sync: PhantomData<*mut ()>,
}

impl UsbContext {
    pub fn new() -> Result<Self, String> {
        let mut raw = ptr::null_mut();
        // SAFETY: libusb_init initializes `raw` or returns an error. No other
        // thread can observe the pointer before this function returns.
        let result = unsafe { libusb_init(&mut raw) };
        if result != 0 {
            return Err(error_name(result));
        }
        if raw.is_null() {
            return Err("libusb_init returned a null context".to_owned());
        }
        Ok(Self {
            raw,
            _not_send_or_sync: PhantomData,
        })
    }

    pub fn target_devices(&self) -> Result<BTreeMap<DeviceKey, DeviceInfo>, String> {
        let mut raw_list = ptr::null_mut();
        // SAFETY: `self.raw` remains valid for the call and `raw_list` points
        // to writable storage for libusb's list pointer.
        let count = unsafe { libusb_get_device_list(self.raw, &mut raw_list) };
        if count < 0 {
            return Err(error_name(count as c_int));
        }
        if raw_list.is_null() {
            return Err("libusb returned a null device list".to_owned());
        }
        let list = DeviceList { raw: raw_list };
        let mut devices = BTreeMap::new();

        for index in 0..count as usize {
            // SAFETY: libusb guarantees `count` initialized device pointers.
            let device = unsafe { *list.raw.add(index) };
            if device.is_null() {
                continue;
            }
            let mut descriptor = LibusbDeviceDescriptor::default();
            // SAFETY: `device` belongs to the live list and `descriptor` is
            // valid writable storage of the ABI-defined type.
            let result = unsafe { libusb_get_device_descriptor(device, &mut descriptor) };
            if result != 0
                || descriptor.id_vendor != VENDOR_ID
                || descriptor.id_product != SM768_PRODUCT_ID
            {
                continue;
            }

            // SAFETY: these accessors only read the live device object.
            let key = DeviceKey {
                bus: unsafe { libusb_get_bus_number(device) },
                address: unsafe { libusb_get_device_address(device) },
            };
            let info = DeviceInfo {
                key,
                // SAFETY: same live-device invariant as above.
                port: unsafe { libusb_get_port_number(device) },
                usb_version: descriptor.bcd_usb,
                device_version: descriptor.bcd_device,
                // SAFETY: same live-device invariant as above.
                speed: Speed::from(unsafe { libusb_get_device_speed(device) }),
            };
            devices.insert(key, info);
        }
        Ok(devices)
    }
}

impl Drop for UsbContext {
    fn drop(&mut self) {
        // SAFETY: `raw` was returned by libusb_init and is released once here,
        // after every DeviceList borrowed from this context has been dropped.
        unsafe { libusb_exit(self.raw) };
    }
}

struct DeviceList {
    raw: *mut *mut LibusbDevice,
}

impl Drop for DeviceList {
    fn drop(&mut self) {
        // SAFETY: `raw` was returned by libusb_get_device_list. Passing 1
        // releases the references owned by the list, exactly once.
        unsafe { libusb_free_device_list(self.raw, 1) };
    }
}

fn error_name(code: c_int) -> String {
    // SAFETY: libusb_error_name returns a process-lifetime string for every
    // integer code; the null check also tolerates a nonconforming library.
    let pointer = unsafe { libusb_error_name(code) };
    if pointer.is_null() {
        format!("libusb error {code}")
    } else {
        // SAFETY: libusb documents the result as a NUL-terminated C string.
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}

pub fn format_bcd(value: u16) -> String {
    format!("{:x}.{:02x}", value >> 8, value & 0xff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn descriptor_matches_libusb_abi() {
        assert_eq!(size_of::<LibusbDeviceDescriptor>(), 18);
        assert_eq!(align_of::<LibusbDeviceDescriptor>(), 2);
    }

    #[test]
    fn speed_values_match_libusb_enum() {
        assert_eq!(Speed::from(3), Speed::High);
        assert_eq!(Speed::from(4), Speed::Super);
        assert_eq!(Speed::from(5), Speed::SuperPlus);
        assert_eq!(Speed::from(99), Speed::Other(99));
    }

    #[test]
    fn bcd_formatter_preserves_nibbles() {
        assert_eq!(format_bcd(0x0320), "3.20");
        assert_eq!(format_bcd(0x0001), "0.01");
    }
}
