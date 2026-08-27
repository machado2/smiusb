#include <libusb-1.0/libusb.h>
#include <stdio.h>

static int expect_no_device(const char *operation, int result) {
    if (result != LIBUSB_ERROR_NO_DEVICE) {
        fprintf(stderr, "%s returned %d (%s), expected LIBUSB_ERROR_NO_DEVICE\n", operation, result,
                libusb_error_name(result));
        return 1;
    }
    return 0;
}

int main(void) {
    unsigned char buffer[8] = {0};
    int transferred = -1;
    int failures = 0;

    failures += expect_no_device(
        "bulk", libusb_bulk_transfer(NULL, 0x02, buffer, sizeof(buffer), &transferred, 10));
    if (transferred != 0) {
        fprintf(stderr, "bulk did not reset transferred to zero\n");
        failures++;
    }
    failures +=
        expect_no_device("interrupt", libusb_interrupt_transfer(NULL, 0x81, buffer, sizeof(buffer),
                                                                &transferred, 10));
    failures += expect_no_device(
        "control", libusb_control_transfer(NULL, 0, 0, 0, 0, buffer, sizeof(buffer), 10));
    failures += expect_no_device("claim", libusb_claim_interface(NULL, 0));
    failures += expect_no_device("release", libusb_release_interface(NULL, 0));
    failures += expect_no_device("reset", libusb_reset_device(NULL));
    libusb_close(NULL);

    return failures == 0 ? 0 : 1;
}
