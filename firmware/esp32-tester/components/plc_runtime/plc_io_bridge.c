#include "plc_io_bridge.h"
#include "plc_vm.h"
#include "board.h"
#include "esp_log.h"
#include "driver/gpio.h"
#include <string.h>
#include <inttypes.h>

static const char *TAG = "plc_io_bridge";

// Waveshare ESP32-S3-Relay-6ch pin mapping
// Index 0-5: relays (GPIO 1, 2, 41, 42, 45, 46)
// Index 6:   buzzer (GPIO 21, LEDC PWM)
// Index 7-9: RGB virtual channels (GPIO 38, WS2812 via RMT)
#define PLC_GPIO_PIN_COUNT  8
#define PLC_GPIO_MAX_ADDRESSES  10
static const gpio_num_t s_gpio_pins[PLC_GPIO_PIN_COUNT] = {
    GPIO_NUM_1,  GPIO_NUM_2,  GPIO_NUM_41, GPIO_NUM_42,
    GPIO_NUM_45, GPIO_NUM_46, GPIO_NUM_21, GPIO_NUM_38,
};

// Current I/O mapping configuration
static plc_io_mapping_t s_mappings[PLC_IO_MAX_MAPPINGS];
static uint32_t s_mapping_count = 0;

esp_err_t plc_io_bridge_init(void) {
    s_mapping_count = 0;
    memset(s_mappings, 0, sizeof(s_mappings));

    // Initialize board peripherals (relays, buzzer PWM, RGB WS2812)
    board_init();

    ESP_LOGI(TAG, "I/O bridge initialized (Waveshare ESP32-S3-Relay-6ch)");
    return ESP_OK;
}

esp_err_t plc_io_bridge_configure(const plc_io_mapping_t *mappings, uint32_t count) {
    if (!mappings || count > PLC_IO_MAX_MAPPINGS) {
        return ESP_ERR_INVALID_ARG;
    }

    memcpy(s_mappings, mappings, count * sizeof(plc_io_mapping_t));
    s_mapping_count = count;

    // Board peripherals already initialized by board_init() in plc_io_bridge_init()
    for (uint32_t i = 0; i < count; i++) {
        const plc_io_mapping_t *m = &s_mappings[i];
        if (m->protocol == PLC_IO_PROTO_GPIO && m->address < PLC_GPIO_MAX_ADDRESSES) {
            ESP_LOGI(TAG, "Mapping var%" PRIu32 " → I/O idx %" PRIu32 " (%s)",
                     m->variable_index, m->address,
                     m->io_type == PLC_IO_INPUT ? "input" : "output");
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
                if (pin_idx < PLC_GPIO_PIN_COUNT) {
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

    // Track RGB components across mappings for a single board_set_rgb() call
    static uint8_t rgb_r = 0, rgb_g = 0, rgb_b = 0;
    static bool rgb_dirty = false;
    rgb_dirty = false;

    for (uint32_t i = 0; i < s_mapping_count; i++) {
        const plc_io_mapping_t *m = &s_mappings[i];
        if (m->io_type != PLC_IO_OUTPUT) continue;
        if (m->variable_index >= vm->variable_count) continue;

        plc_value_t val = vm->variables[m->variable_index];
        int ival = plc_value_to_int(&val);
        bool output = plc_value_to_bool(&val);

        switch (m->protocol) {
            case PLC_IO_PROTO_GPIO: {
                uint32_t pin_idx = m->address;
                if (pin_idx < 6) {
                    // Relay 0-5 (GPIO 1, 2, 41, 42, 45, 46)
                    board_set_relay(pin_idx, output);
                } else if (pin_idx == 6) {
                    // Buzzer (GPIO 21, LEDC PWM)
                    board_set_buzzer(output);
                } else if (pin_idx == 7) {
                    // RGB red component (GPIO 38, WS2812)
                    rgb_r = (uint8_t)(ival & 0xFF);
                    rgb_dirty = true;
                }
                break;
            }
            // RGB green/blue use virtual addresses 8,9 — handled below
            case PLC_IO_PROTO_MODBUS:
            case PLC_IO_PROTO_S7:
            case PLC_IO_PROTO_ENIP:
            case PLC_IO_PROTO_OPCUA:
            case PLC_IO_PROTO_MQTT:
                break;

            default:
                break;
        }
    }

    // Check for green/blue mappings (stored as separate virtual mappings)
    for (uint32_t i = 0; i < s_mapping_count; i++) {
        const plc_io_mapping_t *m = &s_mappings[i];
        if (m->io_type != PLC_IO_OUTPUT) continue;
        if (m->variable_index >= vm->variable_count) continue;
        if (m->protocol != PLC_IO_PROTO_GPIO) continue;
        if (m->address == 8) {
            // RGB green
            plc_value_t val = vm->variables[m->variable_index];
            rgb_g = (uint8_t)(plc_value_to_int(&val) & 0xFF);
            rgb_dirty = true;
        } else if (m->address == 9) {
            // RGB blue
            plc_value_t val = vm->variables[m->variable_index];
            rgb_b = (uint8_t)(plc_value_to_int(&val) & 0xFF);
            rgb_dirty = true;
        }
    }

    if (rgb_dirty) {
        board_set_rgb(rgb_r, rgb_g, rgb_b);
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
