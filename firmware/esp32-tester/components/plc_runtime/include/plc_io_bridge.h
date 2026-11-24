#pragma once

#include <stdint.h>
#include "esp_err.h"
#include "plc_bytecode.h"

// Forward declaration to avoid circular include
typedef struct plc_vm plc_vm_t;

// I/O types
#define PLC_IO_INPUT  0
#define PLC_IO_OUTPUT 1

// Protocol types for I/O mappings
#define PLC_IO_PROTO_NONE   0
#define PLC_IO_PROTO_MODBUS 1
#define PLC_IO_PROTO_S7     2
#define PLC_IO_PROTO_ENIP   3
#define PLC_IO_PROTO_GPIO   4
#define PLC_IO_PROTO_MQTT   5
#define PLC_IO_PROTO_OPCUA  6

// Maximum I/O mappings
#define PLC_IO_MAX_MAPPINGS  128

/**
 * Configure the I/O bridge with a set of mappings.
 * Replaces any existing configuration.
 */
esp_err_t plc_io_bridge_configure(const plc_io_mapping_t *mappings, uint32_t count);

/**
 * Read all mapped inputs into VM variables.
 * Called at the start of each scan cycle.
 * For each input mapping, reads from the protocol handler/GPIO
 * and writes the value into vm->variables[mapping.variable_index].
 */
esp_err_t plc_io_bridge_read_inputs(plc_vm_t *vm);

/**
 * Write all mapped outputs from VM variables.
 * Called at the end of each scan cycle.
 * For each output mapping, reads vm->variables[mapping.variable_index]
 * and writes to the protocol handler/GPIO.
 */
esp_err_t plc_io_bridge_write_outputs(plc_vm_t *vm);

/**
 * Get the current I/O mapping count.
 */
uint32_t plc_io_bridge_get_mapping_count(void);

/**
 * Get a copy of the current I/O mappings.
 * Returns up to max_count mappings into the provided buffer.
 */
uint32_t plc_io_bridge_get_mappings(plc_io_mapping_t *out, uint32_t max_count);

/**
 * Initialize the I/O bridge subsystem.
 */
esp_err_t plc_io_bridge_init(void);
