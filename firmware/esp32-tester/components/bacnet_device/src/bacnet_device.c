#include "bacnet_device.h"
#include "esp_log.h"
#include "board.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"
#include "lwip/netdb.h"
#include "lwip/inet.h"
#include <string.h>
#include <stdlib.h>
#include <errno.h>

static const char *TAG = "bacnet_device";

#define UDP_PORT CONFIG_BACNET_UDP_PORT
#define DEVICE_ID CONFIG_BACNET_DEVICE_ID
#define RX_BUFFER_SIZE 1024
#define TX_BUFFER_SIZE 1024

#define BVLC_TYPE 0x81
#define BVLC_ORIG_UNICAST 0x0A
#define BVLC_ORIG_BROADCAST 0x0B
#define BVLC_FORWARDED 0x04

/* NPDU control flags */
#define NPDU_CONFIRMED_REQUEST 0x04
#define NPDU_UNCONFIRMED_REQUEST 0x00

/* APDU types */
#define BACNET_CONFIRMED_REQUEST 0x00
#define BACNET_UNCONFIRMED_REQUEST 0x10
#define BACNET_SIMPLE_ACK 0x20
#define BACNET_COMPLEX_ACK 0x30
#define BACNET_ERROR 0x50
#define BACNET_REJECT 0x60

/* BACnet Confirmed Services */
#define SERVICE_READ_PROPERTY 0x0C
#define SERVICE_READ_PROPERTY_MULTIPLE 0x0E
#define SERVICE_WHO_IS 0x08

/* BACnet Unconfirmed Services */
#define SERVICE_I_AM 0x00

/* BACnet Object Types */
#define OBJ_DEVICE 0x08
#define OBJ_ANALOG_VALUE 0x02
#define OBJ_BINARY_VALUE 0x04

/* BACnet Property IDs */
#define PROP_OBJECT_IDENTIFIER 0x4C
#define PROP_OBJECT_NAME 0x4D
#define PROP_VENDOR_ID 0x6C
#define PROP_MODEL_NAME 0x70
#define PROP_PRESENT_VALUE 0x55
#define PROP_OBJECT_LIST 0x4F
#define PROP_SYSTEM_STATUS 0x50
#define PROP_DESCRIPTION 0x1C

/* BACnet Application Tag */
#define TAG_NULL 0x00
#define TAG_BOOLEAN 0x01
#define TAG_UNSIGNED_INT 0x02
#define TAG_SIGNED_INT 0x03
#define TAG_REAL 0x04
#define TAG_DOUBLE 0x05
#define TAG_OCTET_STRING 0x06
#define TAG_CHARACTER_STRING 0x07
#define TAG_BIT_STRING 0x08
#define TAG_ENUMERATED 0x09
#define TAG_DATE 0x0A
#define TAG_TIME 0x0B
#define TAG_OBJECT_ID 0x0C

/* BACnet Error Codes */
#define ERROR_UNKNOWN_OBJECT 0x20
#define ERROR_UNKNOWN_PROPERTY 0x20
#define ERROR_READ_ACCESS_DENIED 0x0B

/* Analog Values: Counter(1), Temperature(2), Voltage(3), RGB_R(4), RGB_G(5), RGB_B(6) */
typedef struct {
    const char *name;
    float value;
} analog_value_t;

/* Binary Values: Relay1(1)-Relay6(6), Buzzer(7) */
typedef struct {
    const char *name;
    bool value;
} binary_value_t;

#define NUM_AV 6
#define NUM_BV 7

static analog_value_t av[NUM_AV];
static binary_value_t bv[NUM_BV];
static uint32_t counter = 0;
static bool running = false;

/* ---- Encoding helpers ---- */
static int encode_tag(uint8_t *buf, uint8_t tag_num, bool context, uint32_t len) {
    int pos = 0;
    buf[pos] = (tag_num << 4) | (context ? 0x08 : 0x00);
    if (len < 5) {
        buf[pos] |= (len & 0x07);
        pos++;
    } else {
        buf[pos] |= 0x05;
        pos++;
        /* L-TLV for length > 4 */
        uint8_t l = 0;
        uint32_t tmp = len;
        while (tmp) { l++; tmp >>= 8; }
        buf[pos++] = l;
        for (int i = l - 1; i >= 0; i--) {
            buf[pos++] = (len >> (i * 8)) & 0xFF;
        }
    }
    return pos;
}

static int encode_context_tag(uint8_t *buf, uint8_t tag_num, uint32_t len) {
    return encode_tag(buf, tag_num, true, len);
}

static int encode_app_unsigned(uint8_t *buf, uint32_t val) {
    int pos = 0;
    if (val <= 0xFF) {
        buf[pos++] = (TAG_UNSIGNED_INT << 4) | 0x01;
        buf[pos++] = val & 0xFF;
    } else if (val <= 0xFFFF) {
        buf[pos++] = (TAG_UNSIGNED_INT << 4) | 0x02;
        buf[pos++] = (val >> 8) & 0xFF;
        buf[pos++] = val & 0xFF;
    } else {
        buf[pos++] = (TAG_UNSIGNED_INT << 4) | 0x04;
        buf[pos++] = (val >> 24) & 0xFF;
        buf[pos++] = (val >> 16) & 0xFF;
        buf[pos++] = (val >> 8) & 0xFF;
        buf[pos++] = val & 0xFF;
    }
    return pos;
}

static int encode_app_real(uint8_t *buf, float val) {
    buf[0] = (TAG_REAL << 4) | 0x04;
    uint8_t *fp = (uint8_t *)&val;
    buf[1] = fp[3]; buf[2] = fp[2]; buf[3] = fp[1]; buf[4] = fp[0]; /* big-endian IEEE754 */
    return 5;
}

static int encode_app_boolean(uint8_t *buf, bool val) {
    buf[0] = (TAG_BOOLEAN << 4) | 0x01;
    buf[1] = val ? 0x01 : 0x00;
    return 2;
}

static int encode_app_object_id(uint8_t *buf, uint16_t type, uint32_t instance) {
    uint32_t id = ((uint32_t)type << 22) | (instance & 0x3FFFFF);
    buf[0] = (TAG_OBJECT_ID << 4) | 0x04;
    buf[1] = (id >> 24) & 0xFF;
    buf[2] = (id >> 16) & 0xFF;
    buf[3] = (id >> 8) & 0xFF;
    buf[4] = id & 0xFF;
    return 5;
}

static int encode_app_string(uint8_t *buf, const char *str) {
    int slen = strlen(str);
    int payload_len = slen + 1; /* encoding byte + string */
    int pos = 0;
    if (payload_len < 5) {
        buf[pos++] = (TAG_CHARACTER_STRING << 4) | payload_len;
    } else {
        buf[pos++] = (TAG_CHARACTER_STRING << 4) | 0x05;
        buf[pos++] = payload_len;
    }
    buf[pos++] = 0x00; /* ANSI X3.4 */
    memcpy(&buf[pos], str, slen);
    pos += slen;
    return pos;
}

/* ---- Decode helpers ---- */
static int decode_tag_info(const uint8_t *buf, int buf_len, uint8_t *tag_num, bool *is_context, uint32_t *len) {
    if (buf_len < 1) return -1;
    *tag_num = (buf[0] >> 4) & 0x0F;
    *is_context = (buf[0] & 0x08) ? true : false;
    uint8_t low = buf[0] & 0x07;
    int consumed = 1;
    if (low < 5) {
        *len = low;
    } else if (low == 5) {
        if (buf_len < 2) return -1;
        uint8_t l = buf[1];
        consumed = 2;
        if (buf_len < 2 + l) return -1;
        *len = 0;
        for (int i = 0; i < l; i++) {
            *len = (*len << 8) | buf[consumed++];
        }
    } else {
        *len = 0; /* opening/closing tag */
    }
    return consumed;
}

static uint32_t decode_unsigned(const uint8_t *buf, int len) {
    uint32_t val = 0;
    for (int i = 0; i < len; i++) {
        val = (val << 8) | buf[i];
    }
    return val;
}

static uint32_t decode_object_id(const uint8_t *buf, uint16_t *type, uint32_t *instance) {
    uint32_t id = ((uint32_t)buf[0] << 24) | ((uint32_t)buf[1] << 16) |
                  ((uint32_t)buf[2] << 8) | buf[3];
    *type = (uint16_t)(id >> 22);
    *instance = id & 0x3FFFFF;
    return id;
}

/* ---- Build I-Am response ---- */
static int build_i_am(uint8_t *buf) {
    int pos = 0;

    /* BVLC header */
    buf[pos++] = BVLC_TYPE;
    buf[pos++] = BVLC_ORIG_BROADCAST;
    /* Length placeholder */
    int bvlc_len_pos = pos;
    pos += 2;

    /* NPDU */
    buf[pos++] = 0x01; /* protocol version */
    buf[pos++] = 0x00; /* control = unconfirmed request, no destination */
    /* No SNET/SADR/DNET/DADR/HopCount since control=0 */

    /* APDU: Unconfirmed Request, Service Choice = I-Am (0x00) */
    buf[pos++] = BACNET_UNCONFIRMED_REQUEST | SERVICE_I_AM;

    /* I-Am service: ObjectID, MaxAPDULength, Segmentation, VendorID */
    pos += encode_app_object_id(&buf[pos], OBJ_DEVICE, DEVICE_ID);
    pos += encode_app_unsigned(&buf[pos], 480); /* Max APDU Length */
    pos += encode_app_unsigned(&buf[pos], 0x00); /* Segmentation = none */
    pos += encode_app_unsigned(&buf[pos], 999);  /* Vendor ID */

    /* Update BVLC length */
    uint16_t total_len = pos;
    buf[bvlc_len_pos] = (total_len >> 8) & 0xFF;
    buf[bvlc_len_pos + 1] = total_len & 0xFF;

    return pos;
}

/* ---- Build ReadProperty response ---- */
static int build_read_property_ack(uint8_t *resp, uint8_t invoke_id,
                                    uint16_t obj_type, uint32_t obj_inst,
                                    uint32_t prop_id, const uint8_t *req, int req_len) {
    (void)req; (void)req_len;
    int pos = 0;

    /* BVLC header */
    resp[pos++] = BVLC_TYPE;
    resp[pos++] = BVLC_ORIG_UNICAST;
    int bvlc_len_pos = pos;
    pos += 2;

    /* NPDU */
    resp[pos++] = 0x01; /* protocol version */
    resp[pos++] = 0x00; /* control = unconfirmed (reply, no routing) */

    /* APDU: Complex ACK, Service = ReadProperty (0x0C) */
    resp[pos++] = BACNET_COMPLEX_ACK | SERVICE_READ_PROPERTY;
    resp[pos++] = invoke_id;

    /* ReadPropertyACK: ObjectIdentifier(context 0) + PropertyIdentifier(context 1) + PropertyValue(context 3) */
    /* Object Identifier - context tag 0 */
    pos += encode_context_tag(&resp[pos], 0, 4);
    uint32_t oid = ((uint32_t)obj_type << 22) | (obj_inst & 0x3FFFFF);
    resp[pos++] = (oid >> 24) & 0xFF;
    resp[pos++] = (oid >> 16) & 0xFF;
    resp[pos++] = (oid >> 8) & 0xFF;
    resp[pos++] = oid & 0xFF;

    /* Property Identifier - context tag 1 */
    pos += encode_context_tag(&resp[pos], 1, 1);
    resp[pos++] = prop_id & 0xFF;

    /* Property value - context tag 3 (opening) */
    resp[pos++] = 0x38; /* context tag 3, opening */
    /* Value depends on object and property */
    if (obj_type == OBJ_DEVICE) {
        switch (prop_id) {
        case PROP_OBJECT_IDENTIFIER:
            pos += encode_app_object_id(&resp[pos], OBJ_DEVICE, DEVICE_ID);
            break;
        case PROP_OBJECT_NAME:
            pos += encode_app_string(&resp[pos], "ESP32-BACnet");
            break;
        case PROP_VENDOR_ID:
            pos += encode_app_unsigned(&resp[pos], 999);
            break;
        case PROP_MODEL_NAME:
            pos += encode_app_string(&resp[pos], "ESP32 Tester");
            break;
        case PROP_SYSTEM_STATUS:
            pos += encode_app_unsigned(&resp[pos], 0); /* operational */
            break;
        case PROP_DESCRIPTION:
            pos += encode_app_string(&resp[pos], "ESP32 BACnet/IP Tester Device");
            break;
        case PROP_OBJECT_LIST:
            /* Return count as first element */
            pos += encode_app_unsigned(&resp[pos], 1 + NUM_AV + NUM_BV);
            break;
        default:
            pos += encode_app_unsigned(&resp[pos], 0);
            break;
        }
    } else if (obj_type == OBJ_ANALOG_VALUE) {
        int idx = (int)obj_inst - 1;
        if (idx < 0 || idx >= NUM_AV) idx = 0;
        switch (prop_id) {
        case PROP_OBJECT_IDENTIFIER:
            pos += encode_app_object_id(&resp[pos], OBJ_ANALOG_VALUE, obj_inst);
            break;
        case PROP_OBJECT_NAME:
            pos += encode_app_string(&resp[pos], av[idx].name);
            break;
        case PROP_PRESENT_VALUE:
            pos += encode_app_real(&resp[pos], av[idx].value);
            break;
        case PROP_DESCRIPTION:
            pos += encode_app_string(&resp[pos], av[idx].name);
            break;
        default:
            pos += encode_app_unsigned(&resp[pos], 0);
            break;
        }
    } else if (obj_type == OBJ_BINARY_VALUE) {
        int idx = (int)obj_inst - 1;
        if (idx < 0 || idx >= NUM_BV) idx = 0;
        switch (prop_id) {
        case PROP_OBJECT_IDENTIFIER:
            pos += encode_app_object_id(&resp[pos], OBJ_BINARY_VALUE, obj_inst);
            break;
        case PROP_OBJECT_NAME:
            pos += encode_app_string(&resp[pos], bv[idx].name);
            break;
        case PROP_PRESENT_VALUE:
            pos += encode_app_boolean(&resp[pos], bv[idx].value);
            break;
        case PROP_DESCRIPTION:
            pos += encode_app_string(&resp[pos], bv[idx].name);
            break;
        default:
            pos += encode_app_unsigned(&resp[pos], 0);
            break;
        }
    } else {
        pos += encode_app_unsigned(&resp[pos], 0);
    }

    /* Closing tag for context 3 */
    resp[pos++] = 0x39; /* context tag 3, closing */

    /* Update BVLC length */
    uint16_t total_len = pos;
    resp[bvlc_len_pos] = (total_len >> 8) & 0xFF;
    resp[bvlc_len_pos + 1] = total_len & 0xFF;

    return pos;
}

/* ---- Build Error response ---- */
static int build_error_response(uint8_t *resp, uint8_t invoke_id, uint8_t service, uint8_t error_class, uint8_t error_code) {
    int pos = 0;

    /* BVLC */
    resp[pos++] = BVLC_TYPE;
    resp[pos++] = BVLC_ORIG_UNICAST;
    int bvlc_len_pos = pos;
    pos += 2;

    /* NPDU */
    resp[pos++] = 0x01;
    resp[pos++] = 0x00;

    /* APDU: Error */
    resp[pos++] = BACNET_ERROR | service;
    resp[pos++] = invoke_id;

    /* Error class and error code */
    pos += encode_app_unsigned(&resp[pos], error_class);
    pos += encode_app_unsigned(&resp[pos], error_code);

    /* Update BVLC length */
    uint16_t total_len = pos;
    resp[bvlc_len_pos] = (total_len >> 8) & 0xFF;
    resp[bvlc_len_pos + 1] = total_len & 0xFF;

    return pos;
}

/* ---- Process incoming BACnet message ---- */
static int process_message(uint8_t *rx, int rx_len, struct sockaddr_in *src_addr, uint8_t *tx) {
    if (rx_len < 4) return 0;
    if (rx[0] != BVLC_TYPE) return 0;

    uint8_t bvlc_func = rx[1];
    uint16_t bvlc_len = ((uint16_t)rx[2] << 8) | rx[3];

    if (bvlc_func != BVLC_ORIG_UNICAST && bvlc_func != BVLC_ORIG_BROADCAST && bvlc_func != BVLC_FORWARDED) {
        return 0;
    }

    if (rx_len < (int)bvlc_len || bvlc_len < 4) return 0;

    /* Parse NPDU */
    int npdu_start = 4;
    if (npdu_start >= rx_len) return 0;

    uint8_t npdu_version = rx[npdu_start++];
    if (npdu_version != 0x01) return 0;

    if (npdu_start >= rx_len) return 0;
    uint8_t npdu_control = rx[npdu_start++];

    /* Skip addressing fields based on control */
    if (npdu_control & 0x20) { /* destination specifier */
        npdu_start += 2; /* DNET */
        npdu_start += 1; /* DLEN */
        if (npdu_start < rx_len && rx[npdu_start] != 0)
            npdu_start += rx[npdu_start - 1]; /* DADR */
        npdu_start += 1; /* Hop count */
    }
    if (npdu_control & 0x80) { /* source specifier */
        npdu_start += 2; /* SNET */
        npdu_start += 1; /* SLEN */
        if (npdu_start < rx_len && rx[npdu_start] != 0)
            npdu_start += rx[npdu_start - 1]; /* SADR */
    }

    if (npdu_start >= rx_len) return 0;

    /* Parse APDU */
    uint8_t apdu_type = rx[npdu_start] & 0xF0;
    uint8_t apdu_service = rx[npdu_start] & 0x0F;

    ESP_LOGI(TAG, "APDU type=0x%02X service=0x%02X from %s:%d",
             apdu_type, apdu_service,
             inet_ntoa(src_addr->sin_addr), ntohs(src_addr->sin_port));

    if (apdu_type == BACNET_UNCONFIRMED_REQUEST) {
        /* Unconfirmed request */

        if (apdu_service == SERVICE_WHO_IS) {
            /* Parse optional device-instance-range from Who-Is to check if we should respond */
            int whois_payload_start = npdu_start + 1;
            if (whois_payload_start < rx_len) {
                uint8_t tag_num;
                bool is_ctx;
                uint32_t tlen;
                int consumed = decode_tag_info(&rx[whois_payload_start], rx_len - whois_payload_start, &tag_num, &is_ctx, &tlen);
                if (consumed > 0 && is_ctx && tag_num == 0 && tlen <= 4) {
                    /* There is a device instance range low */
                    uint32_t low = decode_unsigned(&rx[whois_payload_start + consumed], tlen);
                    int next_pos = whois_payload_start + consumed + tlen;
                    consumed = decode_tag_info(&rx[next_pos], rx_len - next_pos, &tag_num, &is_ctx, &tlen);
                    if (consumed > 0 && is_ctx && tag_num == 1 && tlen <= 4) {
                        uint32_t high = decode_unsigned(&rx[next_pos + consumed], tlen);
                        if (DEVICE_ID < low || DEVICE_ID > high) {
                            ESP_LOGI(TAG, "Who-Is range %lu-%lu does not match device %d, ignoring",
                                     (unsigned long)low, (unsigned long)high, DEVICE_ID);
                            return 0;
                        }
                    }
                }
            }
            ESP_LOGI(TAG, "Received Who-Is, sending I-Am to %s:%d",
                     inet_ntoa(src_addr->sin_addr), ntohs(src_addr->sin_port));
            int resp_len = build_i_am(tx);
            /* Override BVLC to unicast back to sender instead of broadcast */
            tx[1] = BVLC_ORIG_UNICAST;
            uint16_t total = resp_len;
            tx[2] = (total >> 8) & 0xFF;
            tx[3] = total & 0xFF;
            return resp_len;
        }
    } else if (apdu_type == BACNET_CONFIRMED_REQUEST) {
        /* Confirmed request: PDU type(1) + MaxSegs(1) + MaxApdu(1) + InvokeID(1) + Service(1+) */
        int service_header = npdu_start + 1;
        if (service_header + 3 >= rx_len) return 0;

        uint8_t max_segs = rx[service_header];
        uint8_t max_apdu = rx[service_header + 1];
        uint8_t invoke_id = rx[service_header + 2];
        uint8_t seg_follows = (max_segs & 0x08) ? 1 : 0;
        (void)max_segs;
        (void)max_apdu;
        (void)seg_follows;

        uint8_t service_choice = rx[service_header + 3];

        if (service_choice == SERVICE_READ_PROPERTY) {
            int rp = service_header + 4;

            /* ReadProperty request: ObjectIdentifier(context 0) + PropertyIdentifier(context 1) + [PropertyArrayIndex(context 2)] */
            uint8_t tag_num;
            bool is_ctx;
            uint32_t tlen;

            /* Object Identifier context tag 0 */
            int consumed = decode_tag_info(&rx[rp], rx_len - rp, &tag_num, &is_ctx, &tlen);
            if (consumed < 0) return 0;
            rp += consumed;
            if (tag_num != 0 || !is_ctx || tlen != 4) {
                return build_error_response(tx, invoke_id, SERVICE_READ_PROPERTY, 1, ERROR_UNKNOWN_OBJECT);
            }
            uint16_t obj_type;
            uint32_t obj_inst;
            decode_object_id(&rx[rp], &obj_type, &obj_inst);
            rp += 4;

            /* Property Identifier context tag 1 */
            consumed = decode_tag_info(&rx[rp], rx_len - rp, &tag_num, &is_ctx, &tlen);
            if (consumed < 0) return 0;
            rp += consumed;
            if (tag_num != 1 || !is_ctx) {
                return build_error_response(tx, invoke_id, SERVICE_READ_PROPERTY, 2, ERROR_UNKNOWN_PROPERTY);
            }
            uint32_t prop_id = decode_unsigned(&rx[rp], tlen);
            rp += tlen;

            ESP_LOGI(TAG, "ReadProperty: obj_type=%u inst=%u prop=%u", (unsigned)obj_type, (unsigned)obj_inst, (unsigned)prop_id);

            /* Validate object */
            bool valid = false;
            if (obj_type == OBJ_DEVICE && obj_inst == DEVICE_ID) valid = true;
            else if (obj_type == OBJ_ANALOG_VALUE && obj_inst >= 1 && obj_inst <= NUM_AV) valid = true;
            else if (obj_type == OBJ_BINARY_VALUE && obj_inst >= 1 && obj_inst <= NUM_BV) valid = true;

            if (!valid) {
                return build_error_response(tx, invoke_id, SERVICE_READ_PROPERTY, 1, ERROR_UNKNOWN_OBJECT);
            }

            return build_read_property_ack(tx, invoke_id, obj_type, obj_inst, prop_id, rx, rx_len);
        } else if (service_choice == SERVICE_READ_PROPERTY_MULTIPLE) {
            /* ReadPropertyMultiple: list of ReadAccessSpecification */
            int rp = service_header + 4;
            int tx_pos = 0;

            /* Build Complex ACK header in tx buffer */
            /* BVLC header */
            tx[tx_pos++] = BVLC_TYPE;
            tx[tx_pos++] = BVLC_ORIG_UNICAST;
            int bvlc_len_pos = tx_pos;
            tx_pos += 2;

            /* NPDU */
            tx[tx_pos++] = 0x01; /* version */
            tx[tx_pos++] = 0x00; /* control */

            /* APDU: Complex ACK for ReadPropertyMultiple */
            tx[tx_pos++] = BACNET_COMPLEX_ACK | SERVICE_READ_PROPERTY_MULTIPLE;
            tx[tx_pos++] = invoke_id;

            /* Parse each ReadAccessSpecification: ObjectIdentifier(context 0) + list of PropertyReference */
            while (rp + 2 < rx_len && tx_pos < TX_BUFFER_SIZE - 64) {
                uint8_t tag_num;
                bool is_ctx;
                uint32_t tlen;

                /* Object Identifier context tag 0 */
                int consumed = decode_tag_info(&rx[rp], rx_len - rp, &tag_num, &is_ctx, &tlen);
                if (consumed < 0) break;
                rp += consumed;
                if (tag_num != 0 || !is_ctx || tlen != 4) break;

                uint16_t obj_type;
                uint32_t obj_inst;
                decode_object_id(&rx[rp], &obj_type, &obj_inst);
                rp += 4;

                /* Validate object */
                bool obj_valid = false;
                if (obj_type == OBJ_DEVICE && obj_inst == DEVICE_ID) obj_valid = true;
                else if (obj_type == OBJ_ANALOG_VALUE && obj_inst >= 1 && obj_inst <= NUM_AV) obj_valid = true;
                else if (obj_type == OBJ_BINARY_VALUE && obj_inst >= 1 && obj_inst <= NUM_BV) obj_valid = true;

                /* Write ReadAccessResult header: ObjectIdentifier(context 0) */
                tx_pos += encode_context_tag(&tx[tx_pos], 0, 4);
                uint32_t oid = ((uint32_t)obj_type << 22) | (obj_inst & 0x3FFFFF);
                tx[tx_pos++] = (oid >> 24) & 0xFF;
                tx[tx_pos++] = (oid >> 16) & 0xFF;
                tx[tx_pos++] = (oid >> 8) & 0xFF;
                tx[tx_pos++] = oid & 0xFF;

                /* Opening tag for list of results (context 1) */
                tx[tx_pos++] = 0x1A; /* context tag 1, opening */

                if (!obj_valid) {
                    /* Unknown object: return error for each requested property */
                    /* Read property list: context tag 0 opening */
                    while (rp + 1 < rx_len) {
                        uint8_t ptag;
                        bool pctx;
                        uint32_t plen;
                        int pc = decode_tag_info(&rx[rp], rx_len - rp, &ptag, &pctx, &plen);
                        if (pc < 0) break;
                        if (ptag == 0 && pctx) {
                            /* Opening tag for property reference list */
                            rp += pc;
                            continue;
                        }
                        if (ptag == 1 && pctx) {
                            /* Closing tag for property reference list */
                            rp += pc;
                            break;
                        }
                        if (ptag == 0 && !pctx) {
                            /* Property identifier */
                            uint32_t pid = decode_unsigned(&rx[rp + pc], plen);
                            rp += pc + plen;
                            /* Check for optional array index */
                            if (rp < rx_len) {
                                uint8_t atag; bool actx; uint32_t alen;
                                int ac = decode_tag_info(&rx[rp], rx_len - rp, &atag, &actx, &alen);
                                if (ac > 0 && atag == 1 && actx) {
                                    rp += ac + alen;
                                }
                            }
                            /* Write error result: PropertyIdentifier(context 2) + opening tag 4 + error + closing tag 4 */
                            tx_pos += encode_context_tag(&tx[tx_pos], 2, 1);
                            tx[tx_pos++] = pid & 0xFF;
                            tx[tx_pos++] = 0x4A; /* context tag 4, opening */
                            /* Error class = OBJECT (1), Error code = UNKNOWN_OBJECT (0x20) */
                            tx_pos += encode_app_unsigned(&tx[tx_pos], 1);
                            tx_pos += encode_app_unsigned(&tx[tx_pos], ERROR_UNKNOWN_OBJECT);
                            tx[tx_pos++] = 0x4B; /* context tag 4, closing */
                            continue;
                        }
                        break;
                    }
                } else {
                    /* Valid object: parse property references and return values */
                    /* The property list starts with opening tag 0 */
                    consumed = decode_tag_info(&rx[rp], rx_len - rp, &tag_num, &is_ctx, &tlen);
                    if (consumed > 0 && tag_num == 0 && is_ctx && tlen == 0 && (rx[rp] & 0x07) == 0x06) {
                        /* Opening tag */
                        rp += consumed;
                    }

                    while (rp + 1 < rx_len && tx_pos < TX_BUFFER_SIZE - 64) {
                        consumed = decode_tag_info(&rx[rp], rx_len - rp, &tag_num, &is_ctx, &tlen);
                        if (consumed < 0) break;

                        /* Check for closing tag (end of property list for this object) */
                        if (tag_num == 1 && is_ctx && tlen == 0 && (rx[rp] & 0x07) == 0x07) {
                            rp += consumed;
                            break;
                        }

                        /* Check for next object (context tag 0) - another ReadAccessSpecification */
                        if (tag_num == 0 && is_ctx) {
                            break;
                        }

                        /* Property identifier (application tag) */
                        uint32_t prop_id = 0;
                        if (!is_ctx) {
                            /* Application-tagged property identifier */
                            prop_id = decode_unsigned(&rx[rp + consumed], tlen);
                            rp += consumed + tlen;

                            /* Check for optional property array index */
                            if (rp < rx_len) {
                                uint8_t atag; bool actx; uint32_t alen;
                                int ac = decode_tag_info(&rx[rp], rx_len - rp, &atag, &actx, &alen);
                                if (ac > 0 && atag == 1 && actx) {
                                    /* Array index present */
                                    rp += ac + alen;
                                }
                            }
                        } else {
                            rp += consumed + tlen;
                            continue;
                        }

                        /* Build result for this property: PropertyIdentifier(context 2) + opening tag 4 + value + closing tag 4 */
                        tx_pos += encode_context_tag(&tx[tx_pos], 2, 1);
                        tx[tx_pos++] = prop_id & 0xFF;
                        tx[tx_pos++] = 0x4E; /* context tag 4, opening, length 1 */

                        /* Encode property value */
                        if (obj_type == OBJ_DEVICE) {
                            switch (prop_id) {
                            case PROP_OBJECT_IDENTIFIER:
                                tx_pos += encode_app_object_id(&tx[tx_pos], OBJ_DEVICE, DEVICE_ID);
                                break;
                            case PROP_OBJECT_NAME:
                                tx_pos += encode_app_string(&tx[tx_pos], "ESP32-BACnet");
                                break;
                            case PROP_VENDOR_ID:
                                tx_pos += encode_app_unsigned(&tx[tx_pos], 999);
                                break;
                            case PROP_MODEL_NAME:
                                tx_pos += encode_app_string(&tx[tx_pos], "ESP32 Tester");
                                break;
                            case PROP_SYSTEM_STATUS:
                                tx_pos += encode_app_unsigned(&tx[tx_pos], 0);
                                break;
                            case PROP_DESCRIPTION:
                                tx_pos += encode_app_string(&tx[tx_pos], "ESP32 BACnet/IP Tester Device");
                                break;
                            case PROP_OBJECT_LIST:
                                tx_pos += encode_app_unsigned(&tx[tx_pos], 1 + NUM_AV + NUM_BV);
                                break;
                            default:
                                tx_pos += encode_app_unsigned(&tx[tx_pos], 0);
                                break;
                            }
                        } else if (obj_type == OBJ_ANALOG_VALUE) {
                            int idx = (int)obj_inst - 1;
                            if (idx < 0 || idx >= NUM_AV) idx = 0;
                            switch (prop_id) {
                            case PROP_OBJECT_IDENTIFIER:
                                tx_pos += encode_app_object_id(&tx[tx_pos], OBJ_ANALOG_VALUE, obj_inst);
                                break;
                            case PROP_OBJECT_NAME:
                                tx_pos += encode_app_string(&tx[tx_pos], av[idx].name);
                                break;
                            case PROP_PRESENT_VALUE:
                                tx_pos += encode_app_real(&tx[tx_pos], av[idx].value);
                                break;
                            default:
                                tx_pos += encode_app_unsigned(&tx[tx_pos], 0);
                                break;
                            }
                        } else if (obj_type == OBJ_BINARY_VALUE) {
                            int idx = (int)obj_inst - 1;
                            if (idx < 0 || idx >= NUM_BV) idx = 0;
                            switch (prop_id) {
                            case PROP_OBJECT_IDENTIFIER:
                                tx_pos += encode_app_object_id(&tx[tx_pos], OBJ_BINARY_VALUE, obj_inst);
                                break;
                            case PROP_OBJECT_NAME:
                                tx_pos += encode_app_string(&tx[tx_pos], bv[idx].name);
                                break;
                            case PROP_PRESENT_VALUE:
                                tx_pos += encode_app_boolean(&tx[tx_pos], bv[idx].value);
                                break;
                            default:
                                tx_pos += encode_app_unsigned(&tx[tx_pos], 0);
                                break;
                            }
                        }

                        tx[tx_pos++] = 0x4B; /* context tag 4, closing */
                    }
                }

                /* Closing tag for list of results (context 1) */
                tx[tx_pos++] = 0x1B; /* context tag 1, closing */
            }

            /* Update BVLC length */
            uint16_t total_len = tx_pos;
            tx[bvlc_len_pos] = (total_len >> 8) & 0xFF;
            tx[bvlc_len_pos + 1] = total_len & 0xFF;

            return tx_pos;
        }
    }

    return 0;
}

/* ---- Data update task ---- */
static void data_update_task(void *arg) {
    while (running) {
        counter++;
        av[0].value = (float)counter;
        av[1].value = 20.0f + ((float)(esp_random() % 15000) / 1000.0f);
        av[2].value = 220.0f + ((float)(esp_random() % 20000) / 1000.0f);

        /* Read relay states from board */
        for (int i = 0; i < 6; i++) {
            bv[i].value = board_get_relay(i);
        }
        /* Buzzer state */
        bv[6].value = false; /* no board_get_buzzer, keep in sync via init */

        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    vTaskDelete(NULL);
}

/* ---- UDP receive task ---- */
static void udp_task(void *arg) {
    struct sockaddr_in dest_addr;
    dest_addr.sin_addr.s_addr = htonl(INADDR_ANY);
    dest_addr.sin_family = AF_INET;
    dest_addr.sin_port = htons(UDP_PORT);

    int sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_IP);
    if (sock < 0) {
        ESP_LOGE(TAG, "socket create failed: errno %d", errno);
        vTaskDelete(NULL);
        return;
    }

    if (bind(sock, (struct sockaddr *)&dest_addr, sizeof(dest_addr)) != 0) {
        ESP_LOGE(TAG, "bind failed: errno %d", errno);
        close(sock);
        vTaskDelete(NULL);
        return;
    }

    ESP_LOGI(TAG, "BACnet Device listening on UDP port %d", UDP_PORT);

    uint8_t *rx_buf = malloc(RX_BUFFER_SIZE);
    uint8_t *tx_buf = malloc(TX_BUFFER_SIZE);
    if (!rx_buf || !tx_buf) {
        ESP_LOGE(TAG, "alloc failed");
        free(rx_buf); free(tx_buf);
        close(sock);
        vTaskDelete(NULL);
        return;
    }

    while (running) {
        struct sockaddr_in src_addr;
        socklen_t addr_len = sizeof(src_addr);
        int received = recvfrom(sock, rx_buf, RX_BUFFER_SIZE, 0,
                                (struct sockaddr *)&src_addr, &addr_len);
        if (received < 0) {
            if (running) ESP_LOGE(TAG, "recvfrom failed: errno %d", errno);
            break;
        }

        int resp_len = process_message(rx_buf, received, &src_addr, tx_buf);
        if (resp_len > 0) {
            sendto(sock, tx_buf, resp_len, 0,
                   (struct sockaddr *)&src_addr, addr_len);
        }
    }

    free(rx_buf);
    free(tx_buf);
    close(sock);
    vTaskDelete(NULL);
}

/* ---- Public API ---- */
esp_err_t bacnet_device_init(void) {
    av[0].name = "Counter";
    av[0].value = 0.0f;
    av[1].name = "Temperature";
    av[1].value = 25.0f;
    av[2].name = "Voltage";
    av[2].value = 230.0f;
    av[3].name = "RGB_R";
    av[3].value = 0.0f;
    av[4].name = "RGB_G";
    av[4].value = 0.0f;
    av[5].name = "RGB_B";
    av[5].value = 0.0f;

    bv[0].name = "Relay1";
    bv[0].value = false;
    bv[1].name = "Relay2";
    bv[1].value = false;
    bv[2].name = "Relay3";
    bv[2].value = false;
    bv[3].name = "Relay4";
    bv[3].value = false;
    bv[4].name = "Relay5";
    bv[4].value = false;
    bv[5].name = "Relay6";
    bv[5].value = false;
    bv[6].name = "Buzzer";
    bv[6].value = false;

    board_init();

    running = true;

    ESP_LOGI(TAG, "BACnet Device initialized (port=%d, device_id=%d)",
             UDP_PORT, DEVICE_ID);
    return ESP_OK;
}

esp_err_t bacnet_device_start(void) {
    xTaskCreate(udp_task, "bacnet_udp", 8192, NULL,
                tskIDLE_PRIORITY + 5, NULL);
    xTaskCreate(data_update_task, "bacnet_data", 3072, NULL,
                tskIDLE_PRIORITY + 3, NULL);
    return ESP_OK;
}
