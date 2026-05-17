#pragma once

#include <stdint.h>
#include <stdbool.h>
#include "plc_vm.h"

// FlatBuffer-like binary encoding helpers
// Custom zero-allocation encoder for ESP32 variable streaming

// Maximum encoded frame size
#define PLC_FLATBUF_MAX_FRAME  8192

/**
 * Encode a variable snapshot into a binary frame for WebSocket transmission.
 * Format:
 *   [1 byte msg_type=0x01]
 *   [2 byte variable_count]
 *   For each variable:
 *     [2 byte name_len] [name_bytes] [1 byte type] [4 byte value] [4 byte timestamp_ms]
 *
 * @param vm         The VM to read variables from
 * @param buf        Output buffer (at least PLC_FLATBUF_MAX_FRAME bytes)
 * @param buf_len    Output buffer size
 * @param out_len    Receives the actual encoded length
 * @return ESP_OK on success
 */
esp_err_t plc_flatbuf_encode_variables(const plc_vm_t *vm, uint8_t *buf, uint32_t buf_len, uint32_t *out_len);

/**
 * Encode a runtime status message.
 * Format:
 *   [1 byte msg_type=0x07]
 *   [1 byte state]
 *   [4 byte cycle_count]
 *   [4 byte scan_time_us]
 *   [4 byte max_scan_time_us]
 *   [4 byte variable_count]
 *
 * @param status  Runtime status
 * @param buf     Output buffer
 * @param buf_len Output buffer size
 * @param out_len Receives the actual encoded length
 */
esp_err_t plc_flatbuf_encode_status(uint8_t state, uint32_t cycle_count,
                                     uint32_t scan_time_us, uint32_t max_scan_time_us,
                                     uint32_t var_count, uint8_t *buf, uint32_t buf_len,
                                     uint32_t *out_len);

/**
 * Encode an error message.
 * Format:
 *   [1 byte msg_type=0x08]
 *   [2 byte msg_len] [msg_bytes]
 */
esp_err_t plc_flatbuf_encode_error(const char *msg, uint8_t *buf, uint32_t buf_len, uint32_t *out_len);

/**
 * Decode a force command from WebSocket frame.
 * Format:
 *   [1 byte msg_type=0x02]
 *   [2 byte name_len] [name_bytes] [1 byte type] [4 byte value]
 *
 * @param data     Incoming binary data
 * @param data_len Data length
 * @param name_out Output buffer for variable name (at least PLC_MAX_NAME_LEN bytes)
 * @param val_out  Output PLC value
 * @return ESP_OK on success
 */
esp_err_t plc_flatbuf_decode_force(const uint8_t *data, uint32_t data_len,
                                    char *name_out, plc_value_t *val_out);

/**
 * Decode an unforce command from WebSocket frame.
 * Format:
 *   [1 byte msg_type=0x03]
 *   [2 byte name_len] [name_bytes]
 */
esp_err_t plc_flatbuf_decode_unforce(const uint8_t *data, uint32_t data_len,
                                      char *name_out);

/**
 * Decode a start command from WebSocket frame.
 * Format:
 *   [1 byte msg_type=0x04]
 *   [4 byte cycle_time_ms]
 */
esp_err_t plc_flatbuf_decode_start(const uint8_t *data, uint32_t data_len,
                                    uint32_t *cycle_time_ms);

// Little-endian read/write helpers (ESP32-S3 is little-endian, but explicit is safer)
static inline void write_u16_le(uint8_t *buf, uint16_t val) {
    buf[0] = (uint8_t)(val & 0xFF);
    buf[1] = (uint8_t)((val >> 8) & 0xFF);
}

static inline void write_u32_le(uint8_t *buf, uint32_t val) {
    buf[0] = (uint8_t)(val & 0xFF);
    buf[1] = (uint8_t)((val >> 8) & 0xFF);
    buf[2] = (uint8_t)((val >> 16) & 0xFF);
    buf[3] = (uint8_t)((val >> 24) & 0xFF);
}

static inline void write_i32_le(uint8_t *buf, int32_t val) {
    write_u32_le(buf, (uint32_t)val);
}

static inline uint16_t read_u16_le(const uint8_t *buf) {
    return (uint16_t)buf[0] | ((uint16_t)buf[1] << 8);
}

static inline uint32_t read_u32_le(const uint8_t *buf) {
    return (uint32_t)buf[0] | ((uint32_t)buf[1] << 8) |
           ((uint32_t)buf[2] << 16) | ((uint32_t)buf[3] << 24);
}

static inline int32_t read_i32_le(const uint8_t *buf) {
    return (int32_t)read_u32_le(buf);
}
