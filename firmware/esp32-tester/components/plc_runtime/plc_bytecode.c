#include "plc_bytecode.h"
#include "plc_flatbuf.h"
#include "esp_log.h"
#include "esp_heap_caps.h"
#include "esp_crc.h"
#include <string.h>
#include <inttypes.h>

static const char *TAG = "plc_bytecode";

static uint32_t crc32_compute(const uint8_t *data, uint32_t len) {
    return esp_crc32_le(0xFFFFFFFF, data, len) ^ 0xFFFFFFFF;
}

esp_err_t plc_bytecode_validate(const uint8_t *data, uint32_t len) {
    if (len < sizeof(plc_bytecode_header_t) + 4) {
        ESP_LOGE(TAG, "bytecode too short: %" PRIu32 " bytes", len);
        return ESP_ERR_INVALID_SIZE;
    }

    const plc_bytecode_header_t *hdr = (const plc_bytecode_header_t *)data;

    if (hdr->magic != PLC_BYTECODE_MAGIC) {
        ESP_LOGE(TAG, "bad magic: 0x%08"PRIX32" expected 0x%08"PRIX32, hdr->magic, (uint32_t)PLC_BYTECODE_MAGIC);
        return ESP_ERR_INVALID_CRC;
    }

    if (hdr->version > PLC_BYTECODE_VERSION) {
        ESP_LOGE(TAG, "unsupported version: %" PRIu16, hdr->version);
        return ESP_ERR_INVALID_VERSION;
    }

    if (hdr->instruction_count > PLC_MAX_INSTRUCTIONS) {
        ESP_LOGE(TAG, "too many instructions: %" PRIu32 " (max %d)", hdr->instruction_count, PLC_MAX_INSTRUCTIONS);
        return ESP_ERR_INVALID_SIZE;
    }

    if (hdr->variable_count > PLC_MAX_VARIABLES) {
        ESP_LOGE(TAG, "too many variables: %" PRIu32 " (max %d)", hdr->variable_count, PLC_MAX_VARIABLES);
        return ESP_ERR_INVALID_SIZE;
    }

    if (hdr->io_mapping_count > PLC_MAX_IO_MAPPINGS) {
        ESP_LOGE(TAG, "too many I/O mappings: %" PRIu32 " (max %d)", hdr->io_mapping_count, PLC_MAX_IO_MAPPINGS);
        return ESP_ERR_INVALID_SIZE;
    }

    // Verify CRC (last 4 bytes)
    uint32_t computed_crc = crc32_compute(data, len - 4);
    uint32_t stored_crc = read_u32_le(data + len - 4);
    if (computed_crc != stored_crc) {
        ESP_LOGE(TAG, "CRC mismatch: computed 0x%08" PRIX32 " stored 0x%08" PRIX32, computed_crc, stored_crc);
        return ESP_ERR_INVALID_CRC;
    }

    return ESP_OK;
}

esp_err_t plc_bytecode_load(const uint8_t *data, uint32_t len, plc_bytecode_t *out) {
    memset(out, 0, sizeof(*out));

    esp_err_t ret = plc_bytecode_validate(data, len);
    if (ret != ESP_OK) {
        return ret;
    }

    const plc_bytecode_header_t *hdr = (const plc_bytecode_header_t *)data;
    const uint8_t *ptr = data + sizeof(plc_bytecode_header_t);
    const uint8_t *end = data + len - 4;  // Reserve 4 bytes for CRC

    // Bounds check helper macro
    #define CHECK_BOUNDS(extra) do { \
        if (ptr + (extra) > end) { \
            ESP_LOGE(TAG, "bytecode section overflows buffer at offset %ld", (long)(ptr - data)); \
            plc_bytecode_free(out); \
            return ESP_ERR_INVALID_SIZE; \
        } \
    } while(0)

    // Allocate instructions
    if (hdr->instruction_count > 0) {
        CHECK_BOUNDS(hdr->instruction_count * 6);
        out->instructions = heap_caps_malloc(hdr->instruction_count * sizeof(plc_instruction_t), MALLOC_CAP_8BIT);
        if (!out->instructions) {
            ESP_LOGE(TAG, "failed to alloc %" PRIu32 " instructions", hdr->instruction_count);
            plc_bytecode_free(out);
            return ESP_ERR_NO_MEM;
        }
        out->instruction_count = hdr->instruction_count;

        for (uint32_t i = 0; i < hdr->instruction_count; i++) {
            out->instructions[i].opcode = *ptr++;
            out->instructions[i].has_operand = *ptr++;
            out->instructions[i].operand = (int32_t)read_u32_le(ptr);
            ptr += 4;
        }
    }

    // Allocate variable init values
    if (hdr->variable_count > 0) {
        CHECK_BOUNDS(hdr->variable_count * 5);  // Each var: 1 type + 4 value
        out->variable_init = heap_caps_malloc(hdr->variable_count * sizeof(plc_value_t), MALLOC_CAP_8BIT);
        out->variable_names = heap_caps_malloc(hdr->variable_count * PLC_MAX_NAME_LEN, MALLOC_CAP_8BIT);
        if (!out->variable_init || !out->variable_names) {
            ESP_LOGE(TAG, "failed to alloc %" PRIu32 " variables", hdr->variable_count);
            plc_bytecode_free(out);
            return ESP_ERR_NO_MEM;
        }
        out->variable_count = hdr->variable_count;
        memset(out->variable_names, 0, hdr->variable_count * PLC_MAX_NAME_LEN);

        for (uint32_t i = 0; i < hdr->variable_count; i++) {
            out->variable_init[i].type = (plc_value_type_t)*ptr++;
            uint32_t raw_val = read_u32_le(ptr);
            ptr += 4;

            switch (out->variable_init[i].type) {
                case PLC_TYPE_BOOL:   out->variable_init[i].bool_val = (raw_val != 0); break;
                case PLC_TYPE_BYTE:   out->variable_init[i].byte_val = (uint8_t)raw_val; break;
                case PLC_TYPE_INT:    out->variable_init[i].int_val = (int16_t)raw_val; break;
                case PLC_TYPE_DINT:   out->variable_init[i].dint_val = (int32_t)raw_val; break;
                case PLC_TYPE_REAL:   memcpy(&out->variable_init[i].real_val, &raw_val, 4); break;
                case PLC_TYPE_TIME:   out->variable_init[i].time_val = (int32_t)raw_val; break;
                case PLC_TYPE_STRING: out->variable_init[i].str_idx = (uint16_t)raw_val; break;
                case PLC_TYPE_WORD:   out->variable_init[i].word_val = (uint16_t)raw_val; break;
                case PLC_TYPE_DWORD:  out->variable_init[i].dword_val = raw_val; break;
            }
        }

        // Read variable names
        for (uint32_t i = 0; i < hdr->variable_count; i++) {
            CHECK_BOUNDS(1);
            uint8_t name_len = *ptr++;
            CHECK_BOUNDS(name_len);
            if (name_len >= PLC_MAX_NAME_LEN) name_len = PLC_MAX_NAME_LEN - 1;
            memcpy(out->variable_names[i], ptr, name_len);
            out->variable_names[i][name_len] = '\0';
            ptr += name_len;
        }
    }

    // Allocate I/O mappings
    if (hdr->io_mapping_count > 0) {
        CHECK_BOUNDS(hdr->io_mapping_count * 10);  // Each mapping: 4 + 1 + 1 + 4
        out->io_mappings = heap_caps_malloc(hdr->io_mapping_count * sizeof(plc_io_mapping_t), MALLOC_CAP_8BIT);
        if (!out->io_mappings) {
            ESP_LOGE(TAG, "failed to alloc %" PRIu32 " I/O mappings", hdr->io_mapping_count);
            plc_bytecode_free(out);
            return ESP_ERR_NO_MEM;
        }
        out->io_mapping_count = hdr->io_mapping_count;

        for (uint32_t i = 0; i < hdr->io_mapping_count; i++) {
            out->io_mappings[i].variable_index = read_u32_le(ptr); ptr += 4;
            out->io_mappings[i].io_type = *ptr++;
            out->io_mappings[i].protocol = *ptr++;
            out->io_mappings[i].address = read_u32_le(ptr); ptr += 4;
        }
    }

    // Read name constants
    if (hdr->name_constant_count > 0) {
        CHECK_BOUNDS(hdr->name_constant_count);  // At least 1 byte per name len
        out->name_constants = heap_caps_malloc(hdr->name_constant_count * PLC_MAX_NAME_LEN, MALLOC_CAP_8BIT);
        if (!out->name_constants) {
            ESP_LOGE(TAG, "failed to alloc %" PRIu32 " name constants", hdr->name_constant_count);
            plc_bytecode_free(out);
            return ESP_ERR_NO_MEM;
        }
        out->name_constant_count = hdr->name_constant_count;
        memset(out->name_constants, 0, hdr->name_constant_count * PLC_MAX_NAME_LEN);

        for (uint32_t i = 0; i < hdr->name_constant_count; i++) {
            CHECK_BOUNDS(1);
            uint8_t name_len = *ptr++;
            CHECK_BOUNDS(name_len);
            if (name_len >= PLC_MAX_NAME_LEN) name_len = PLC_MAX_NAME_LEN - 1;
            memcpy(out->name_constants[i], ptr, name_len);
            out->name_constants[i][name_len] = '\0';
            ptr += name_len;
        }
    }

    ESP_LOGI(TAG, "loaded bytecode: %" PRIu32 " instr, %" PRIu32 " vars, %" PRIu32 " io, %" PRIu32 " names",
             hdr->instruction_count, hdr->variable_count,
             hdr->io_mapping_count, hdr->name_constant_count);

    return ESP_OK;
}

void plc_bytecode_free(plc_bytecode_t *bc) {
    if (bc->instructions) {
        free(bc->instructions);
        bc->instructions = NULL;
    }
    if (bc->variable_init) {
        free(bc->variable_init);
        bc->variable_init = NULL;
    }
    if (bc->variable_names) {
        free(bc->variable_names);
        bc->variable_names = NULL;
    }
    if (bc->io_mappings) {
        free(bc->io_mappings);
        bc->io_mappings = NULL;
    }
    if (bc->name_constants) {
        free(bc->name_constants);
        bc->name_constants = NULL;
    }
    bc->instruction_count = 0;
    bc->variable_count = 0;
    bc->io_mapping_count = 0;
    bc->name_constant_count = 0;
}
