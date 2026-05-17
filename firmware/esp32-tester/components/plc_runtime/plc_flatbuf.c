#include "plc_flatbuf.h"
#include "plc_ws_server.h"
#include "esp_log.h"
#include "esp_timer.h"
#include <string.h>
#include <inttypes.h>

static const char *TAG = "plc_flatbuf";

esp_err_t plc_flatbuf_encode_variables(const plc_vm_t *vm, uint8_t *buf, uint32_t buf_len, uint32_t *out_len) {
    if (!vm || !buf || !out_len) return ESP_ERR_INVALID_ARG;

    uint32_t pos = 0;

    // Message type
    buf[pos++] = PLC_WS_MSG_VAR_UPDATE;

    // Variable count (2 bytes LE)
    uint16_t var_count = (uint16_t)vm->variable_count;
    write_u16_le(buf + pos, var_count);
    pos += 2;

    // Encode each variable
    for (uint16_t i = 0; i < var_count; i++) {
        // Check buffer space (need at least: 2 + name + 1 + 4 + 4 = 11 + name_len)
        const char *name = plc_vm_get_variable_name(vm, i);
        uint16_t name_len = 0;
        if (name) {
            name_len = (uint16_t)strlen(name);
            if (name_len > 30) name_len = 30;  // Truncate long names
        }

        uint32_t needed = 2 + name_len + 1 + 4 + 4;
        if (pos + needed > buf_len) {
            ESP_LOGW(TAG, "frame truncated at variable %u", i);
            // Update variable count to actual encoded count
            write_u16_le(buf + 1, i);
            break;
        }

        // Name length (2 bytes)
        write_u16_le(buf + pos, name_len);
        pos += 2;

        // Name bytes
        if (name_len > 0) {
            memcpy(buf + pos, name, name_len);
            pos += name_len;
        }

        // Value type (1 byte)
        const plc_value_t *val = plc_vm_get_variable(vm, i);
        if (val) {
            buf[pos++] = (uint8_t)val->type;

            // Value (4 bytes, raw union)
            switch (val->type) {
                case PLC_TYPE_BOOL:
                    write_u32_le(buf + pos, val->bool_val ? 1 : 0);
                    break;
                case PLC_TYPE_BYTE:
                    write_u32_le(buf + pos, val->byte_val);
                    break;
                case PLC_TYPE_INT:
                    write_i32_le(buf + pos, val->int_val);
                    break;
                case PLC_TYPE_DINT:
                    write_i32_le(buf + pos, val->dint_val);
                    break;
                case PLC_TYPE_REAL:
                    memcpy(buf + pos, &val->real_val, 4);
                    break;
                case PLC_TYPE_TIME:
                    write_i32_le(buf + pos, val->time_val);
                    break;
                case PLC_TYPE_STRING:
                    write_u16_le(buf + pos, val->str_idx);
                    buf[pos + 2] = 0;
                    buf[pos + 3] = 0;
                    break;
                case PLC_TYPE_WORD:
                    write_u32_le(buf + pos, val->word_val);
                    break;
                case PLC_TYPE_DWORD:
                    write_u32_le(buf + pos, val->dword_val);
                    break;
                default:
                    write_u32_le(buf + pos, 0);
                    break;
            }
            pos += 4;
        } else {
            buf[pos++] = 0;
            write_u32_le(buf + pos, 0);
            pos += 4;
        }

        // Timestamp (4 bytes, milliseconds since boot, truncated)
        int64_t now_us = esp_timer_get_time();
        uint32_t now_ms = (uint32_t)(now_us / 1000);
        write_u32_le(buf + pos, now_ms);
        pos += 4;
    }

    *out_len = pos;
    return ESP_OK;
}

esp_err_t plc_flatbuf_encode_status(uint8_t state, uint32_t cycle_count,
                                     uint32_t scan_time_us, uint32_t max_scan_time_us,
                                     uint32_t var_count, uint8_t *buf, uint32_t buf_len,
                                     uint32_t *out_len) {
    if (!buf || !out_len) return ESP_ERR_INVALID_ARG;
    if (buf_len < 20) return ESP_ERR_INVALID_SIZE;

    uint32_t pos = 0;
    buf[pos++] = PLC_WS_MSG_STATUS;
    buf[pos++] = state;
    write_u32_le(buf + pos, cycle_count);     pos += 4;
    write_u32_le(buf + pos, scan_time_us);    pos += 4;
    write_u32_le(buf + pos, max_scan_time_us); pos += 4;
    write_u32_le(buf + pos, var_count);       pos += 4;

    *out_len = pos;
    return ESP_OK;
}

esp_err_t plc_flatbuf_encode_error(const char *msg, uint8_t *buf, uint32_t buf_len, uint32_t *out_len) {
    if (!buf || !out_len) return ESP_ERR_INVALID_ARG;
    if (!msg) msg = "";

    uint16_t msg_len = (uint16_t)strlen(msg);
    if (msg_len > 200) msg_len = 200;

    uint32_t needed = 1 + 2 + msg_len;
    if (buf_len < needed) return ESP_ERR_INVALID_SIZE;

    uint32_t pos = 0;
    buf[pos++] = PLC_WS_MSG_ERROR;
    write_u16_le(buf + pos, msg_len);
    pos += 2;
    memcpy(buf + pos, msg, msg_len);
    pos += msg_len;

    *out_len = pos;
    return ESP_OK;
}

esp_err_t plc_flatbuf_decode_force(const uint8_t *data, uint32_t data_len,
                                    char *name_out, plc_value_t *val_out) {
    if (!data || !name_out || !val_out) return ESP_ERR_INVALID_ARG;

    // Expect: [0x02] [2-byte name_len] [name_bytes] [1-byte type] [4-byte value]
    uint32_t pos = 0;

    if (data_len < 4) return ESP_ERR_INVALID_SIZE;
    if (data[pos++] != PLC_WS_MSG_FORCE) return ESP_ERR_INVALID_ARG;

    uint16_t name_len = read_u16_le(data + pos);
    pos += 2;

    if (pos + name_len + 5 > data_len) return ESP_ERR_INVALID_SIZE;

    // Copy name
    uint16_t copy_len = (name_len < PLC_MAX_NAME_LEN - 1) ? name_len : PLC_MAX_NAME_LEN - 1;
    memcpy(name_out, data + pos, copy_len);
    name_out[copy_len] = '\0';
    pos += name_len;

    // Read type and value
    val_out->type = (plc_value_type_t)data[pos++];
    uint32_t raw_val = read_u32_le(data + pos);

    switch (val_out->type) {
        case PLC_TYPE_BOOL:   val_out->bool_val = (raw_val != 0); break;
        case PLC_TYPE_BYTE:   val_out->byte_val = (uint8_t)raw_val; break;
        case PLC_TYPE_INT:    val_out->int_val = (int16_t)raw_val; break;
        case PLC_TYPE_DINT:   val_out->dint_val = (int32_t)raw_val; break;
        case PLC_TYPE_REAL:   memcpy(&val_out->real_val, &raw_val, 4); break;
        case PLC_TYPE_TIME:   val_out->time_val = (int32_t)raw_val; break;
        case PLC_TYPE_STRING: val_out->str_idx = (uint16_t)raw_val; break;
        case PLC_TYPE_WORD:   val_out->word_val = (uint16_t)raw_val; break;
        case PLC_TYPE_DWORD:  val_out->dword_val = raw_val; break;
    }

    return ESP_OK;
}

esp_err_t plc_flatbuf_decode_unforce(const uint8_t *data, uint32_t data_len, char *name_out) {
    if (!data || !name_out) return ESP_ERR_INVALID_ARG;

    uint32_t pos = 0;
    if (data_len < 4) return ESP_ERR_INVALID_SIZE;
    if (data[pos++] != PLC_WS_MSG_UNFORCE) return ESP_ERR_INVALID_ARG;

    uint16_t name_len = read_u16_le(data + pos);
    pos += 2;

    if (pos + name_len > data_len) return ESP_ERR_INVALID_SIZE;

    uint16_t copy_len = (name_len < PLC_MAX_NAME_LEN - 1) ? name_len : PLC_MAX_NAME_LEN - 1;
    memcpy(name_out, data + pos, copy_len);
    name_out[copy_len] = '\0';

    return ESP_OK;
}

esp_err_t plc_flatbuf_decode_start(const uint8_t *data, uint32_t data_len, uint32_t *cycle_time_ms) {
    if (!data || !cycle_time_ms) return ESP_ERR_INVALID_ARG;

    if (data_len < 5) return ESP_ERR_INVALID_SIZE;
    if (data[0] != PLC_WS_MSG_START) return ESP_ERR_INVALID_ARG;

    *cycle_time_ms = read_u32_le(data + 1);
    return ESP_OK;
}
