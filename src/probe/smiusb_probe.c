#include <libusb-1.0/libusb.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

enum {
    SMI_VENDOR_ID = 0x090c,
    SMI_SM768_PRODUCT_ID = 0x0768,
};

static volatile sig_atomic_t stop_requested;

static void request_stop(int signal_number) {
    (void)signal_number;
    stop_requested = 1;
}

static void print_endpoint(const struct libusb_endpoint_descriptor *endpoint) {
    const char *direction =
        (endpoint->bEndpointAddress & LIBUSB_ENDPOINT_DIR_MASK) != 0 ? "IN" : "OUT";
    const char *type = "unknown";

    switch (endpoint->bmAttributes & LIBUSB_TRANSFER_TYPE_MASK) {
    case LIBUSB_TRANSFER_TYPE_CONTROL:
        type = "control";
        break;
    case LIBUSB_TRANSFER_TYPE_ISOCHRONOUS:
        type = "isochronous";
        break;
    case LIBUSB_TRANSFER_TYPE_BULK:
        type = "bulk";
        break;
    case LIBUSB_TRANSFER_TYPE_INTERRUPT:
        type = "interrupt";
        break;
    default:
        break;
    }

    printf("      endpoint=0x%02x direction=%s type=%s max_packet=%u interval=%u\n",
           endpoint->bEndpointAddress, direction, type, endpoint->wMaxPacketSize,
           endpoint->bInterval);
}

static void print_device(libusb_device *device, const char *event) {
    struct libusb_device_descriptor descriptor;
    struct libusb_config_descriptor *config = NULL;
    int result = libusb_get_device_descriptor(device, &descriptor);

    if (result != LIBUSB_SUCCESS) {
        fprintf(stderr, "descriptor read failed: %s\n", libusb_error_name(result));
        return;
    }
    if (descriptor.idVendor != SMI_VENDOR_ID || descriptor.idProduct != SMI_SM768_PRODUCT_ID) {
        return;
    }

    printf("%s bus=%u address=%u vid:pid=%04x:%04x usb=%x.%02x "
           "device=%x.%02x configurations=%u\n",
           event, libusb_get_bus_number(device), libusb_get_device_address(device),
           descriptor.idVendor, descriptor.idProduct, descriptor.bcdUSB >> 8,
           descriptor.bcdUSB & 0xff, descriptor.bcdDevice >> 8, descriptor.bcdDevice & 0xff,
           descriptor.bNumConfigurations);

    for (uint8_t config_index = 0; config_index < descriptor.bNumConfigurations; ++config_index) {
        result = libusb_get_config_descriptor(device, config_index, &config);
        if (result != LIBUSB_SUCCESS) {
            fprintf(stderr, "configuration %u read failed: %s\n", config_index,
                    libusb_error_name(result));
            continue;
        }

        printf("  config=%u value=%u interfaces=%u attributes=0x%02x power=%umA\n", config_index,
               config->bConfigurationValue, config->bNumInterfaces, config->bmAttributes,
               (unsigned int)config->MaxPower * 2U);
        for (uint8_t interface_index = 0; interface_index < config->bNumInterfaces;
             ++interface_index) {
            const struct libusb_interface *interface = &config->interface[interface_index];
            for (int alternate_index = 0; alternate_index < interface->num_altsetting;
                 ++alternate_index) {
                const struct libusb_interface_descriptor *alternate =
                    &interface->altsetting[alternate_index];
                printf("    interface=%u alt=%u class=%02x/%02x/%02x endpoints=%u\n",
                       alternate->bInterfaceNumber, alternate->bAlternateSetting,
                       alternate->bInterfaceClass, alternate->bInterfaceSubClass,
                       alternate->bInterfaceProtocol, alternate->bNumEndpoints);
                for (uint8_t endpoint_index = 0; endpoint_index < alternate->bNumEndpoints;
                     ++endpoint_index) {
                    print_endpoint(&alternate->endpoint[endpoint_index]);
                }
            }
        }
        libusb_free_config_descriptor(config);
        config = NULL;
    }
    fflush(stdout);
}

static int hotplug_callback(libusb_context *context, libusb_device *device,
                            libusb_hotplug_event event, void *user_data) {
    (void)context;
    (void)user_data;
    print_device(device, event == LIBUSB_HOTPLUG_EVENT_DEVICE_ARRIVED ? "arrived" : "left");
    return 0;
}

static int enumerate(libusb_context *context) {
    libusb_device **devices = NULL;
    ssize_t count = libusb_get_device_list(context, &devices);

    if (count < 0) {
        fprintf(stderr, "USB enumeration failed: %s\n", libusb_error_name((int)count));
        return 1;
    }
    for (ssize_t index = 0; index < count; ++index) {
        print_device(devices[index], "present");
    }
    libusb_free_device_list(devices, 1);
    return 0;
}

int main(int argc, char **argv) {
    libusb_context *context = NULL;
    libusb_hotplug_callback_handle callback_handle;
    int watch_seconds = 0;
    int result;

    if (argc == 3 && strcmp(argv[1], "--watch") == 0) {
        watch_seconds = atoi(argv[2]);
        if (watch_seconds < 1) {
            fprintf(stderr, "--watch requires a positive number of seconds\n");
            return 2;
        }
    } else if (argc != 1) {
        fprintf(stderr, "usage: %s [--watch SECONDS]\n", argv[0]);
        return 2;
    }

    result = libusb_init(&context);
    if (result != LIBUSB_SUCCESS) {
        fprintf(stderr, "libusb_init failed: %s\n", libusb_error_name(result));
        return 1;
    }

    result = enumerate(context);
    if (result != 0 || watch_seconds == 0) {
        libusb_exit(context);
        return result;
    }

    signal(SIGINT, request_stop);
    signal(SIGTERM, request_stop);
    result = libusb_hotplug_register_callback(
        context, LIBUSB_HOTPLUG_EVENT_DEVICE_ARRIVED | LIBUSB_HOTPLUG_EVENT_DEVICE_LEFT,
        LIBUSB_HOTPLUG_NO_FLAGS, SMI_VENDOR_ID, SMI_SM768_PRODUCT_ID, LIBUSB_HOTPLUG_MATCH_ANY,
        hotplug_callback, NULL, &callback_handle);
    if (result != LIBUSB_SUCCESS) {
        fprintf(stderr, "hotplug registration failed: %s\n", libusb_error_name(result));
        libusb_exit(context);
        return 1;
    }

    time_t deadline = time(NULL) + watch_seconds;
    while (!stop_requested && time(NULL) < deadline) {
        struct timeval timeout = {.tv_sec = 0, .tv_usec = 250000};
        result = libusb_handle_events_timeout_completed(context, &timeout, NULL);
        if (result != LIBUSB_SUCCESS && result != LIBUSB_ERROR_INTERRUPTED) {
            fprintf(stderr, "event loop failed: %s\n", libusb_error_name(result));
            break;
        }
    }

    libusb_hotplug_deregister_callback(context, callback_handle);
    libusb_exit(context);
    return result == LIBUSB_SUCCESS || result == LIBUSB_ERROR_INTERRUPTED ? 0 : 1;
}
