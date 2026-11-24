#pragma once

#include <stdint.h>
#include "esp_err.h"
#include "plc_vm.h"
#include "plc_io_bridge.h"

// Runtime state enum (public API)
typedef enum {
    PLC_STATE_STOPPED = 0,
    PLC_STATE_RUNNING,
    PLC_STATE_PAUSED,
    PLC_STATE_ERROR,
} plc_state_t;

// Runtime statistics
typedef struct {
    plc_state_t state;
    uint32_t cycle_count;
    uint32_t scan_time_us;
    uint32_t max_scan_time_us;
    uint32_t avg_scan_time_us;
    uint32_t variable_count;
    // CRC32 of the loaded bundle body (drift detection), 0 = nothing loaded.
    uint32_t loaded_bundle_crc32;
} plc_runtime_status_t;

/**
 * Initialize the PLC runtime subsystem.
 * Creates the VM, I/O bridge, but does not start scanning.
 * Must be called before any other plc_runtime_* function.
 */
esp_err_t plc_runtime_init(void);

/**
 * Start the scan cycle task.
 * The task runs on Core 1 at the specified cycle rate.
 * @param cycle_time_ms  Scan cycle period in milliseconds (1-10000)
 */
esp_err_t plc_runtime_start(uint32_t cycle_time_ms);

/**
 * Stop the scan cycle task. VM state is preserved.
 */
esp_err_t plc_runtime_stop(void);

/**
 * Execute a single scan cycle (step mode).
 * Only valid when runtime is stopped or paused.
 */
esp_err_t plc_runtime_step(void);

/**
 * Load bytecode into the runtime VM.
 * Stops any running scan cycle first.
 */
esp_err_t plc_runtime_load_bytecode(const uint8_t *data, uint32_t len);

/**
 * Get the current runtime status.
 */
plc_runtime_status_t plc_runtime_get_status(void);

/**
 * Get pointer to the internal VM (for direct variable access).
 * The VM may be modified by the scan task concurrently.
 * Use only when runtime is stopped, or for read-only access.
 */
plc_vm_t *plc_runtime_get_vm(void);

/**
 * Get current runtime state.
 */
plc_state_t plc_runtime_get_state(void);

/**
 * Get cycle count.
 */
uint32_t plc_runtime_get_cycle_count(void);

/**
 * Get last scan time in microseconds.
 */
uint32_t plc_runtime_get_scan_time_us(void);

/**
 * Force a variable by index.
 */
esp_err_t plc_runtime_force_variable(uint32_t var_index, const plc_value_t *val);

/**
 * Force a variable by name. Thread-safe (takes runtime mutex).
 */
esp_err_t plc_runtime_force_variable_by_name(const char *name, const plc_value_t *val);

/**
 * Unforce a variable by index (releases force, variable resumes normal behavior).
 */
esp_err_t plc_runtime_unforce_variable(uint32_t var_index);

/**
 * Unforce a variable by name. Thread-safe (takes runtime mutex).
 */
esp_err_t plc_runtime_unforce_variable_by_name(const char *name);

/**
 * Build a JSON object string of all variable name→value pairs.
 * Thread-safe (takes runtime mutex).
 * @param buf     Output buffer for JSON string
 * @param buf_len Size of output buffer
 * @return Number of bytes written (excluding null terminator)
 */
int plc_runtime_get_variables_json(char *buf, int buf_len);
