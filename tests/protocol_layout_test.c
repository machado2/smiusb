#include "smiusb/protocol.h"

#include <stdio.h>
#include <string.h>

int main(void) {
    struct smiusb_sm768_header header = {0};

    memcpy(header.prefix.magic, SMIUSB_WIRE_MAGIC, SMIUSB_WIRE_MAGIC_SIZE);
    if (memcmp(header.prefix.magic, "smifalconsta", SMIUSB_WIRE_MAGIC_SIZE) != 0) {
        fprintf(stderr, "SMIUSB wire magic layout mismatch\n");
        return 1;
    }
    if (sizeof(header) != SMIUSB_SM768_HEADER_SIZE) {
        fprintf(stderr, "SM768 header layout mismatch\n");
        return 1;
    }
    return 0;
}
