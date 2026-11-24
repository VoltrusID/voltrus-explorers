#pragma once

#include <stdint.h>
#include <stdbool.h>
#include "esp_err.h"
#include "plc_bytecode.h"

// VM stack and memory limits for ESP32-S3
#define PLC_VM_STACK_SIZE       256
#define PLC_VM_CALL_STACK_SIZE  32
#define PLC_VM_MAX_TIMERS       16
#define PLC_VM_MAX_COUNTERS     16
#define PLC_VM_MAX_EDGE_REGS    16

// Timer kinds
typedef enum {
    PLC_TIMER_TON = 0,
    PLC_TIMER_TOF = 1,
    PLC_TIMER_TP  = 2,
} plc_timer_kind_t;

// Timer instance state
typedef struct {
    plc_timer_kind_t kind;
    bool     in_prev;
    bool     q;
    int32_t  et;         // elapsed time in ms
    int32_t  pt;         // preset time in ms
    int64_t  start_tick; // esp_timer_get_time() when timer started
    bool     timing;     // timer is actively running
} plc_timer_t;

// Counter kinds
typedef enum {
    PLC_COUNTER_CTU  = 0,
    PLC_COUNTER_CTD  = 1,
    PLC_COUNTER_CTUD = 2,
} plc_counter_kind_t;

// Counter instance state
typedef struct {
    plc_counter_kind_t kind;
    bool  cu_prev;
    bool  cd_prev;
    int32_t cv;   // current value
    bool  q;      // output
} plc_counter_t;

// VM state enum
typedef enum {
    PLC_VM_STOPPED = 0,
    PLC_VM_RUNNING,
    PLC_VM_PAUSED,
    PLC_VM_ERROR,
    PLC_VM_BREAKPOINT_HIT,
} plc_vm_state_t;

// Breakpoint set (compact bitmap for up to PLC_MAX_INSTRUCTIONS)
#define PLC_BP_BITMAP_SIZE ((PLC_MAX_INSTRUCTIONS + 31) / 32)

// Forced variable set (bitmap for up to PLC_MAX_VARIABLES)
#define PLC_FORCED_BITMAP_SIZE ((PLC_MAX_VARIABLES + 31) / 32)

// Main VM struct
typedef struct plc_vm {
    // Execution state
    plc_vm_state_t state;
    uint32_t pc;                  // Program counter
    uint32_t cycle_count;         // Scan cycles completed
    uint32_t scan_time_us;        // Last scan time in microseconds
    uint32_t max_scan_time_us;    // Peak scan time
    uint32_t total_scan_time_us;  // Cumulative (for avg)
    char     error_msg[128];      // Last error message

    // Instructions
    plc_instruction_t *instructions;
    uint32_t instruction_count;

    // Variable memory
    plc_value_t *variables;
    char (*variable_names)[PLC_MAX_NAME_LEN];
    uint32_t variable_count;

    // Evaluation stack
    plc_value_t stack[PLC_VM_STACK_SIZE];
    uint32_t stack_ptr;

    // Call stack (for POU calls)
    uint32_t call_stack[PLC_VM_CALL_STACK_SIZE];
    uint32_t call_stack_ptr;

    // Name constants (for string/enum lookup)
    char (*name_constants)[PLC_MAX_NAME_LEN];
    uint32_t name_constant_count;

    // Function block state
    plc_timer_t   timers[PLC_VM_MAX_TIMERS];
    plc_counter_t counters[PLC_VM_MAX_COUNTERS];
    bool          edge_prev[PLC_VM_MAX_EDGE_REGS];

    // Breakpoints (bitmap)
    uint32_t breakpoints[PLC_BP_BITMAP_SIZE];

    // Forced variables (bitmap — when set, StoreVar is skipped for that slot)
    uint32_t forced_vars[PLC_FORCED_BITMAP_SIZE];

    // Single-step mode
    bool single_step;
} plc_vm_t;

/**
 * Initialize VM with default values. Does not allocate.
 * Call plc_vm_load_bytecode() after to load a program.
 */
void plc_vm_init(plc_vm_t *vm);

/**
 * Load a validated bytecode bundle into the VM.
 * Allocates memory for instructions and variables.
 * Resets VM state.
 */
esp_err_t plc_vm_load_bytecode(plc_vm_t *vm, const plc_bytecode_t *bc);

/**
 * Execute one complete scan cycle (all instructions from PC=0 to Halt/end).
 * Returns ESP_OK on success, ESP_FAIL on runtime error.
 * Updates vm->state, vm->scan_time_us, vm->cycle_count.
 */
esp_err_t plc_vm_execute_cycle(plc_vm_t *vm);

/**
 * Reset VM to initial state (variables re-initialized, timers/counters cleared).
 * Does not unload bytecode.
 */
void plc_vm_reset(plc_vm_t *vm);

/**
 * Free all memory allocated for instructions and variables.
 */
void plc_vm_cleanup(plc_vm_t *vm);

/**
 * Get variable value by index.
 */
const plc_value_t *plc_vm_get_variable(const plc_vm_t *vm, uint32_t index);

/**
 * Get variable name by index. Returns NULL if out of range.
 */
const char *plc_vm_get_variable_name(const plc_vm_t *vm, uint32_t index);

/**
 * Set variable value by index.
 */
esp_err_t plc_vm_set_variable(plc_vm_t *vm, uint32_t index, const plc_value_t *val);

/**
 * Force a variable value (same as set, but marks as forced).
 */
esp_err_t plc_vm_force_variable(plc_vm_t *vm, uint32_t index, const plc_value_t *val);
esp_err_t plc_vm_unforce_variable(plc_vm_t *vm, uint32_t index);
bool plc_vm_is_variable_forced(const plc_vm_t *vm, uint32_t index);

/**
 * Set/clear breakpoint at instruction index.
 */
void plc_vm_set_breakpoint(plc_vm_t *vm, uint32_t pc);
void plc_vm_clear_breakpoint(plc_vm_t *vm, uint32_t pc);

/**
 * Check if breakpoint is set at pc.
 */
bool plc_vm_has_breakpoint(const plc_vm_t *vm, uint32_t pc);

// --- Value conversion helpers ---

/** Convert any plc_value_t to bool */
bool plc_value_to_bool(const plc_value_t *v);

/** Convert any plc_value_t to int32 */
int32_t plc_value_to_int(const plc_value_t *v);

/** Convert any plc_value_t to float */
float plc_value_to_real(const plc_value_t *v);

/** Convert any plc_value_t to string (writes into provided buffer) */
void plc_value_to_string(const plc_value_t *v, const char *name_constants, uint32_t name_constant_count,
                         char *buf, uint32_t buf_len);
