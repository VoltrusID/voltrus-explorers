#include "plc_io_bridge.h"
#include "plc_vm.h"
#include "esp_log.h"
#include "driver/gpio.h"
#include <string.h>
#include <inttypes.h>

static const char *TAG = "plc_io_bridge";

// GPIO pins available on ESP32-S3 for I/O (adjust per board)
#define PLC_GPIO_MAX  16
static const gpio_num_t s_gpio_pins[PLC_GPIO_MAX] = {
    GPIO_NUM_1,  GPIO_NUM_2,  GPIO_NUM_3,  GPIO_NUM_4,
    GPIO_NUM_5,  GPIO_NUM_6,  GPIO_NUM_7,  GPIO_NUM_8,
    GPIO_NUM_9,  GPIO_NUM_10, GPIO_NUM_11, GPIO_NUM_12,
    GPIO_NUM_13, GPIO_NUM_14, GPIO_NUM_15, GPIO_NUM_16,
};

// Current I/O mapping configuration
static plc_io_mapping_t s_mappings[PLC_IO_MAX_MAPPINGS];
static uint32_t s_mapping_count = 0;

esp_err_t plc_io_bridge_init(void) {
    s_mapping_count = 0;
    memset(s_mappings, 0, sizeof(s_mappings));

    // Configure all GPIO pins as inactive (will be configured per-mapping)
    for (int i = 0; i < PLC_GPIO_MAX; i++) {
        gpio_reset_pin(s_gpio_pins[i]);
    }

    ESP_LOGI(TAG, "I/O bridge initialized");
    return ESP_OK;
}

esp_err_t plc_io_bridge_configure(const plc_io_mapping_t *mappings, uint32_t count) {
    if (!mappings || count > PLC_IO_MAX_MAPPINGS) {
        return ESP_ERR_INVALID_ARG;
    }

    memcpy(s_mappings, mappings, count * sizeof(plc_io_mapping_t));
    s_mapping_count = count;

    // Configure GPIO pins based on mappings
    for (uint32_t i = 0; i < count; i++) {
        const plc_io_mapping_t *m = &s_mappings[i];
        if (m->protocol == PLC_IO_PROTO_GPIO) {
            uint32_t pin_idx = m->address;
            if (pin_idx < PLC_GPIO_MAX) {
                gpio_num_t pin = s_gpio_pins[pin_idx];
                if (m->io_type == PLC_IO_INPUT) {
                    gpio_set_direction(pin, GPIO_MODE_INPUT);
                    gpio_set_pull_mode(pin, GPIO_PULLUP_ONLY);
                } else {
                    gpio_set_direction(pin, GPIO_MODE_OUTPUT);
                    gpio_set_level(pin, 0);
                }
                ESP_LOGI(TAG, "GPIO%" PRIu32 " configured as %s (pin=%d, var=%" PRIu32 ")",
                         pin_idx, m->io_type == PLC_IO_INPUT ? "input" : "output",
                         pin, m->variable_index);
            }
        }
    }

    ESP_LOGI(TAG, "I/O bridge configured: %" PRIu32 " mappings", count);
    return ESP_OK;
}

esp_err_t plc_io_bridge_read_inputs(plc_vm_t *vm) {
    if (!vm || !vm->variables) return ESP_ERR_INVALID_ARG;

    for (uint32_t i = 0; i < s_mapping_count; i++) {
        const plc_io_mapping_t *m = &s_mappings[i];
        if (m->io_type != PLC_IO_INPUT) continue;
        if (m->variable_index >= vm->variable_count) continue;

        plc_value_t val = { .type = PLC_TYPE_BOOL, .bool_val = false };

        switch (m->protocol) {
            case PLC_IO_PROTO_GPIO: {
                uint32_t pin_idx = m->address;
                if (pin_idx < PLC_GPIO_MAX) {
                    val.bool_val = (gpio_get_level(s_gpio_pins[pin_idx]) != 0);
                }
                break;
            }
            case PLC_IO_PROTO_MODBUS:
            case PLC_IO_PROTO_S7:
            case PLC_IO_PROTO_ENIP:
            case PLC_IO_PROTO_OPCUA:
            case PLC_IO_PROTO_MQTT:
                // Protocol driver reads would go here
                // For now, these are stubs -- the protocol components can register
                // callback functions that the I/O bridge calls
                break;

            default:
                break;
        }

        vm->variables[m->variable_index] = val;
    }

    return ESP_OK;
}

esp_err_t plc_io_bridge_write_outputs(plc_vm_t *vm) {
    if (!vm || !vm->variables) return ESP_ERR_INVALID_ARG;

    for (uint32_t i = 0; i < s_mapping_count; i++) {
        const plc_io_mapping_t *m = &s_mappings[i];
        if (m->io_type != PLC_IO_OUTPUT) continue;
        if (m->variable_index >= vm->variable_count) continue;

        plc_value_t val = vm->variables[m->variable_index];
        bool output = plc_value_to_bool(&val);

        switch (m->protocol) {
            case PLC_IO_PROTO_GPIO: {
                uint32_t pin_idx = m->address;
                if (pin_idx < PLC_GPIO_MAX) {
                    gpio_set_level(s_gpio_pins[pin_idx], output ? 1 : 0);
                }
                break;
            }
            case PLC_IO_PROTO_MODBUS:
            case PLC_IO_PROTO_S7:
            case PLC_IO_PROTO_ENIP:
            case PLC_IO_PROTO_OPCUA:
            case PLC_IO_PROTO_MQTT:
                // Protocol driver writes would go here
                break;

            default:
                break;
        }
    }

    return ESP_OK;
}

uint32_t plc_io_bridge_get_mapping_count(void) {
    return s_mapping_count;
}

uint32_t plc_io_bridge_get_mappings(plc_io_mapping_t *out, uint32_t max_count) {
    if (!out) return 0;
    uint32_t count = (max_count < s_mapping_count) ? max_count : s_mapping_count;
    memcpy(out, s_mappings, count * sizeof(plc_io_mapping_t));
    return count;
}
