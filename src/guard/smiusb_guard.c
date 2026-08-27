#include <dlfcn.h>
#include <libusb-1.0/libusb.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/*
 * The vendor daemon can race device removal against its detached init thread
 * and call libusb with a NULL handle. libusb expects a valid handle and
 * dereferences its internal mutex. Returning an error alone is insufficient:
 * the worker continues through a global map entry erased by the detach thread.
 * Interpose at the public ABI boundary and, for the vendor service, turn USB
 * removal into a controlled process restart before unsafe cleanup can run.
 */

static atomic_ulong blocked_calls = 0;
static atomic_int restart_on_disconnect = 0;

struct guarded_hotplug_callback {
    libusb_hotplug_callback_fn callback;
    void *user_data;
};

static bool restart_fence_enabled(void) {
    int cached = atomic_load_explicit(&restart_on_disconnect, memory_order_acquire);
    if (cached != 0) {
        return cached == 2;
    }

    const char *value = getenv("SMIUSB_GUARD_RESTART_ON_DISCONNECT");
    bool enabled = value != NULL && value[0] != '\0' && strcmp(value, "0") != 0;
    atomic_store_explicit(&restart_on_disconnect, enabled ? 2 : 1, memory_order_release);
    return enabled;
}

static void controlled_restart(const char *reason) {
    char message[256];
    int length =
        snprintf(message, sizeof(message), "smiusb-guard: controlled daemon restart: %s\n", reason);
    if (length > 0) {
        size_t bytes = (size_t)length;
        if (bytes > sizeof(message)) {
            bytes = sizeof(message);
        }
        (void)write(STDERR_FILENO, message, bytes);
    }

    /*
     * The vendor's detach callback destroys an object while detached workers
     * still use it. Process exit is the only safe lifetime boundary available
     * through LD_PRELOAD; systemd restarts the daemon immediately.
     */
    _exit(0);
}

static void restart_after_usb_disconnect(const char *function_name, int result) {
    if (result == LIBUSB_ERROR_NO_DEVICE && restart_fence_enabled()) {
        controlled_restart(function_name);
    }
}

static void guard_log(const char *function_name) {
    unsigned long count = atomic_fetch_add_explicit(&blocked_calls, 1, memory_order_relaxed) + 1;
    char message[256];
    int length = snprintf(message, sizeof(message),
                          "smiusb-guard: blocked %s with NULL handle "
                          "(returning LIBUSB_ERROR_NO_DEVICE, count=%lu)\n",
                          function_name, count);

    if (length > 0) {
        size_t bytes = (size_t)length;
        if (bytes > sizeof(message)) {
            bytes = sizeof(message);
        }
        (void)write(STDERR_FILENO, message, bytes);
    }
}

static void resolve_or_die(void **target, const char *symbol) {
    const char *error;

    (void)dlerror();
    *target = dlsym(RTLD_NEXT, symbol);
    error = dlerror();
    if (*target == NULL || error != NULL) {
        char message[256];
        int length = snprintf(message, sizeof(message), "smiusb-guard: cannot resolve %s: %s\n",
                              symbol, error != NULL ? error : "unknown dynamic linker error");
        if (length > 0) {
            size_t bytes = (size_t)length;
            if (bytes > sizeof(message)) {
                bytes = sizeof(message);
            }
            (void)write(STDERR_FILENO, message, bytes);
        }
        _exit(127);
    }
}

#define DECLARE_REAL(name, type)                                                                   \
    static type real_##name;                                                                       \
    static pthread_once_t once_##name = PTHREAD_ONCE_INIT;                                         \
    static void resolve_##name(void) { resolve_or_die((void **)&real_##name, #name); }

typedef int (*bulk_transfer_fn)(libusb_device_handle *, unsigned char, unsigned char *, int, int *,
                                unsigned int);
typedef int (*control_transfer_fn)(libusb_device_handle *, uint8_t, uint8_t, uint16_t, uint16_t,
                                   unsigned char *, uint16_t, unsigned int);
typedef int (*interface_fn)(libusb_device_handle *, int);
typedef int (*alt_setting_fn)(libusb_device_handle *, int, int);
typedef int (*reset_fn)(libusb_device_handle *);
typedef void (*close_fn)(libusb_device_handle *);
typedef int (*hotplug_register_fn)(libusb_context *, int, int, int, int, int,
                                   libusb_hotplug_callback_fn, void *,
                                   libusb_hotplug_callback_handle *);

DECLARE_REAL(libusb_bulk_transfer, bulk_transfer_fn)
DECLARE_REAL(libusb_interrupt_transfer, bulk_transfer_fn)
DECLARE_REAL(libusb_control_transfer, control_transfer_fn)
DECLARE_REAL(libusb_claim_interface, interface_fn)
DECLARE_REAL(libusb_release_interface, interface_fn)
DECLARE_REAL(libusb_set_interface_alt_setting, alt_setting_fn)
DECLARE_REAL(libusb_detach_kernel_driver, interface_fn)
DECLARE_REAL(libusb_reset_device, reset_fn)
DECLARE_REAL(libusb_close, close_fn)
DECLARE_REAL(libusb_hotplug_register_callback, hotplug_register_fn)

static int guarded_hotplug_dispatch(libusb_context *context, libusb_device *device,
                                    libusb_hotplug_event event, void *user_data) {
    struct guarded_hotplug_callback *guarded = user_data;

    if (event == LIBUSB_HOTPLUG_EVENT_DEVICE_LEFT && restart_fence_enabled()) {
        struct libusb_device_descriptor descriptor;
        if (libusb_get_device_descriptor(device, &descriptor) == LIBUSB_SUCCESS &&
            descriptor.idVendor == 0x090c && descriptor.idProduct == 0x0768) {
            controlled_restart("SM768 hotplug detach");
        }
    }

    return guarded->callback(context, device, event, guarded->user_data);
}

int libusb_hotplug_register_callback(libusb_context *context, int events, int flags, int vendor_id,
                                     int product_id, int device_class,
                                     libusb_hotplug_callback_fn callback, void *user_data,
                                     libusb_hotplug_callback_handle *callback_handle) {
    pthread_once(&once_libusb_hotplug_register_callback, resolve_libusb_hotplug_register_callback);

    if (callback == NULL) {
        return real_libusb_hotplug_register_callback(context, events, flags, vendor_id, product_id,
                                                     device_class, callback, user_data,
                                                     callback_handle);
    }

    struct guarded_hotplug_callback *guarded = malloc(sizeof(*guarded));
    if (guarded == NULL) {
        return LIBUSB_ERROR_NO_MEM;
    }
    guarded->callback = callback;
    guarded->user_data = user_data;

    int result = real_libusb_hotplug_register_callback(
        context, events, flags, vendor_id, product_id, device_class, guarded_hotplug_dispatch,
        guarded, callback_handle);
    if (result != LIBUSB_SUCCESS) {
        free(guarded);
    }
    return result;
}

int libusb_bulk_transfer(libusb_device_handle *handle, unsigned char endpoint, unsigned char *data,
                         int length, int *transferred, unsigned int timeout) {
    if (handle == NULL) {
        if (transferred != NULL) {
            *transferred = 0;
        }
        guard_log(__func__);
        restart_after_usb_disconnect(__func__, LIBUSB_ERROR_NO_DEVICE);
        return LIBUSB_ERROR_NO_DEVICE;
    }
    pthread_once(&once_libusb_bulk_transfer, resolve_libusb_bulk_transfer);
    int result = real_libusb_bulk_transfer(handle, endpoint, data, length, transferred, timeout);
    restart_after_usb_disconnect(__func__, result);
    return result;
}

int libusb_interrupt_transfer(libusb_device_handle *handle, unsigned char endpoint,
                              unsigned char *data, int length, int *transferred,
                              unsigned int timeout) {
    if (handle == NULL) {
        if (transferred != NULL) {
            *transferred = 0;
        }
        guard_log(__func__);
        restart_after_usb_disconnect(__func__, LIBUSB_ERROR_NO_DEVICE);
        return LIBUSB_ERROR_NO_DEVICE;
    }
    pthread_once(&once_libusb_interrupt_transfer, resolve_libusb_interrupt_transfer);
    int result =
        real_libusb_interrupt_transfer(handle, endpoint, data, length, transferred, timeout);
    restart_after_usb_disconnect(__func__, result);
    return result;
}

int libusb_control_transfer(libusb_device_handle *handle, uint8_t request_type, uint8_t request,
                            uint16_t value, uint16_t index, unsigned char *data, uint16_t length,
                            unsigned int timeout) {
    if (handle == NULL) {
        guard_log(__func__);
        restart_after_usb_disconnect(__func__, LIBUSB_ERROR_NO_DEVICE);
        return LIBUSB_ERROR_NO_DEVICE;
    }
    pthread_once(&once_libusb_control_transfer, resolve_libusb_control_transfer);
    int result = real_libusb_control_transfer(handle, request_type, request, value, index, data,
                                              length, timeout);
    restart_after_usb_disconnect(__func__, result);
    return result;
}

#define NULL_GUARD_INTERFACE(name)                                                                 \
    int name(libusb_device_handle *handle, int interface_number) {                                 \
        if (handle == NULL) {                                                                      \
            guard_log(__func__);                                                                   \
            return LIBUSB_ERROR_NO_DEVICE;                                                         \
        }                                                                                          \
        pthread_once(&once_##name, resolve_##name);                                                \
        return real_##name(handle, interface_number);                                              \
    }

NULL_GUARD_INTERFACE(libusb_claim_interface)
NULL_GUARD_INTERFACE(libusb_release_interface)
NULL_GUARD_INTERFACE(libusb_detach_kernel_driver)

int libusb_set_interface_alt_setting(libusb_device_handle *handle, int interface_number,
                                     int alternate_setting) {
    if (handle == NULL) {
        guard_log(__func__);
        return LIBUSB_ERROR_NO_DEVICE;
    }
    pthread_once(&once_libusb_set_interface_alt_setting, resolve_libusb_set_interface_alt_setting);
    return real_libusb_set_interface_alt_setting(handle, interface_number, alternate_setting);
}

int libusb_reset_device(libusb_device_handle *handle) {
    if (handle == NULL) {
        guard_log(__func__);
        return LIBUSB_ERROR_NO_DEVICE;
    }
    pthread_once(&once_libusb_reset_device, resolve_libusb_reset_device);
    return real_libusb_reset_device(handle);
}

void libusb_close(libusb_device_handle *handle) {
    if (handle == NULL) {
        guard_log(__func__);
        return;
    }
    pthread_once(&once_libusb_close, resolve_libusb_close);
    real_libusb_close(handle);
}
