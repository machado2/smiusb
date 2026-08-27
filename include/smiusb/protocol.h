#ifndef SMIUSB_PROTOCOL_H
#define SMIUSB_PROTOCOL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum {
    SMIUSB_VENDOR_ID = 0x090c,
    SMIUSB_SM768_PRODUCT_ID = 0x0768,
    SMIUSB_WIRE_MAGIC_SIZE = 12,
    SMIUSB_SM768_HEADER_SIZE = 48,
    SMIUSB_SM768_JPEG_DECODER_HEADER_SIZE = 0x5000,
    SMIUSB_SM768_CAPABILITIES_RESPONSE = 0x65,
    SMIUSB_SM768_OBSERVED_BULK_OUT_ENDPOINT = 0x02,
    SMIUSB_SM768_OBSERVED_INTERRUPT_IN_ENDPOINT = 0x81,
    SMIUSB_SM768_OBSERVED_HEARTBEAT_SIZE = 44,
    SMIUSB_SM768_OBSERVED_INTERRUPT_READ_SIZE = 1024,
};

#define SMIUSB_WIRE_MAGIC "smifalconsta"

#if defined(__GNUC__) || defined(__clang__)
#define SMIUSB_PACKED __attribute__((packed))
#else
#define SMIUSB_PACKED
#endif

/*
 * The word after the magic is packet-specific. It is a transfer length for
 * observed frame packets, but the initial capability packet places an ASCII
 * discriminator there. Keep it opaque until captures confirm every variant.
 */
struct SMIUSB_PACKED smiusb_wire_prefix {
    uint8_t magic[SMIUSB_WIRE_MAGIC_SIZE];
    uint32_t packet_word_le;
};

struct SMIUSB_PACKED smiusb_sm768_header {
    struct smiusb_wire_prefix prefix;
    uint8_t packet_specific[SMIUSB_SM768_HEADER_SIZE - sizeof(struct smiusb_wire_prefix)];
};

#ifdef __cplusplus
}

static_assert(sizeof(smiusb_wire_prefix) == 16);
static_assert(sizeof(smiusb_sm768_header) == SMIUSB_SM768_HEADER_SIZE);
#else
_Static_assert(sizeof(struct smiusb_wire_prefix) == 16, "unexpected wire prefix padding");
_Static_assert(sizeof(struct smiusb_sm768_header) == SMIUSB_SM768_HEADER_SIZE,
               "unexpected SM768 header padding");
#endif

#endif
