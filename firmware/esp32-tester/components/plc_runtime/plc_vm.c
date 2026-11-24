#include "plc_vm.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "esp_heap_caps.h"
#include <string.h>
#include <math.h>
#include <stdlib.h>
#include <inttypes.h>

static const char *TAG = "plc_vm";

// --- Value conversion helpers ---

bool plc_value_to_bool(const plc_value_t *v) {
    if (!v) return false;
    switch (v->type) {
        case PLC_TYPE_BOOL:   return v->bool_val;
        case PLC_TYPE_BYTE:   return v->byte_val != 0;
        case PLC_TYPE_INT:    return v->int_val != 0;
        case PLC_TYPE_DINT:   return v->dint_val != 0;
        case PLC_TYPE_REAL:   return v->real_val != 0.0f;
        case PLC_TYPE_TIME:   return v->time_val != 0;
        case PLC_TYPE_STRING: return v->str_idx != 0;
        case PLC_TYPE_WORD:   return v->word_val != 0;
        case PLC_TYPE_DWORD:  return v->dword_val != 0;
        default: return false;
    }
}

int32_t plc_value_to_int(const plc_value_t *v) {
    if (!v) return 0;
    switch (v->type) {
        case PLC_TYPE_BOOL:   return v->bool_val ? 1 : 0;
        case PLC_TYPE_BYTE:   return (int32_t)v->byte_val;
        case PLC_TYPE_INT:    return (int32_t)v->int_val;
        case PLC_TYPE_DINT:   return v->dint_val;
        case PLC_TYPE_REAL:   return (int32_t)v->real_val;
        case PLC_TYPE_TIME:   return v->time_val;
        case PLC_TYPE_WORD:   return (int32_t)v->word_val;
        case PLC_TYPE_DWORD:  return (int32_t)v->dword_val;
        case PLC_TYPE_STRING: return 0;
        default: return 0;
    }
}

float plc_value_to_real(const plc_value_t *v) {
    if (!v) return 0.0f;
    switch (v->type) {
        case PLC_TYPE_BOOL:   return v->bool_val ? 1.0f : 0.0f;
        case PLC_TYPE_BYTE:   return (float)v->byte_val;
        case PLC_TYPE_INT:    return (float)v->int_val;
        case PLC_TYPE_DINT:   return (float)v->dint_val;
        case PLC_TYPE_REAL:   return v->real_val;
        case PLC_TYPE_TIME:   return (float)v->time_val;
        case PLC_TYPE_WORD:   return (float)v->word_val;
        case PLC_TYPE_DWORD:  return (float)v->dword_val;
        case PLC_TYPE_STRING: return 0.0f;
        default: return 0.0f;
    }
}

void plc_value_to_string(const plc_value_t *v, const char *name_constants,
                         uint32_t name_constant_count, char *buf, uint32_t buf_len) {
    if (!v || !buf || buf_len == 0) {
        if (buf && buf_len > 0) buf[0] = '\0';
        return;
    }
    switch (v->type) {
        case PLC_TYPE_BOOL:
            snprintf(buf, buf_len, "%s", v->bool_val ? "TRUE" : "FALSE");
            break;
        case PLC_TYPE_BYTE:
            snprintf(buf, buf_len, "%u", v->byte_val);
            break;
        case PLC_TYPE_INT:
            snprintf(buf, buf_len, "%d", v->int_val);
            break;
        case PLC_TYPE_DINT:
            snprintf(buf, buf_len, "%ld", (long)v->dint_val);
            break;
        case PLC_TYPE_REAL:
            snprintf(buf, buf_len, "%.3f", v->real_val);
            break;
        case PLC_TYPE_TIME:
            snprintf(buf, buf_len, "%ldms", (long)v->time_val);
            break;
        case PLC_TYPE_STRING:
            if (name_constants && v->str_idx < name_constant_count) {
                snprintf(buf, buf_len, "%s",
                         &name_constants[v->str_idx * PLC_MAX_NAME_LEN]);
            } else {
                buf[0] = '\0';
            }
            break;
        case PLC_TYPE_WORD:
            snprintf(buf, buf_len, "%u", v->word_val);
            break;
        case PLC_TYPE_DWORD:
            snprintf(buf, buf_len, "%lu", (unsigned long)v->dword_val);
            break;
        default:
            buf[0] = '\0';
            break;
    }
}

// --- Stack operations ---

static inline bool stack_push(plc_vm_t *vm, plc_value_t val) {
    if (vm->stack_ptr >= PLC_VM_STACK_SIZE) {
        ESP_LOGE(TAG, "stack overflow at pc=%" PRIu32, vm->pc);
        return false;
    }
    vm->stack[vm->stack_ptr++] = val;
    return true;
}

static inline bool stack_pop(plc_vm_t *vm, plc_value_t *out) {
    if (vm->stack_ptr == 0) {
        ESP_LOGE(TAG, "stack underflow at pc=%" PRIu32, vm->pc);
        return false;
    }
    *out = vm->stack[--vm->stack_ptr];
    return true;
}

// --- VM Init/Cleanup ---

void plc_vm_init(plc_vm_t *vm) {
    memset(vm, 0, sizeof(*vm));
    vm->state = PLC_VM_STOPPED;
    vm->pc = 0;
    vm->cycle_count = 0;
    vm->stack_ptr = 0;
    vm->call_stack_ptr = 0;
    vm->single_step = false;
}

esp_err_t plc_vm_load_bytecode(plc_vm_t *vm, const plc_bytecode_t *bc) {
    // Free previous allocations
    plc_vm_cleanup(vm);

    // Clear forced variable bitmap
    memset(vm->forced_vars, 0, sizeof(vm->forced_vars));

    // Copy instructions
    if (bc->instruction_count > 0 && bc->instructions) {
        vm->instructions = heap_caps_malloc(bc->instruction_count * sizeof(plc_instruction_t), MALLOC_CAP_8BIT);
        if (!vm->instructions) {
            ESP_LOGE(TAG, "failed to alloc instructions");
            return ESP_ERR_NO_MEM;
        }
        memcpy(vm->instructions, bc->instructions, bc->instruction_count * sizeof(plc_instruction_t));
        vm->instruction_count = bc->instruction_count;
    }

    // Allocate and initialize variables
    if (bc->variable_count > 0 && bc->variable_init) {
        vm->variables = heap_caps_malloc(bc->variable_count * sizeof(plc_value_t), MALLOC_CAP_8BIT);
        if (!vm->variables) {
            ESP_LOGE(TAG, "failed to alloc variables");
            plc_vm_cleanup(vm);
            return ESP_ERR_NO_MEM;
        }
        memcpy(vm->variables, bc->variable_init, bc->variable_count * sizeof(plc_value_t));
        vm->variable_count = bc->variable_count;

        // Copy variable names
        vm->variable_names = heap_caps_malloc(bc->variable_count * PLC_MAX_NAME_LEN, MALLOC_CAP_8BIT);
        if (!vm->variable_names) {
            ESP_LOGE(TAG, "failed to alloc variable names");
            plc_vm_cleanup(vm);
            return ESP_ERR_NO_MEM;
        }
        memcpy(vm->variable_names, bc->variable_names, bc->variable_count * PLC_MAX_NAME_LEN);
    }

    // Copy name constants
    if (bc->name_constant_count > 0 && bc->name_constants) {
        vm->name_constants = heap_caps_malloc(bc->name_constant_count * PLC_MAX_NAME_LEN, MALLOC_CAP_8BIT);
        if (!vm->name_constants) {
            ESP_LOGE(TAG, "failed to alloc name constants");
            plc_vm_cleanup(vm);
            return ESP_ERR_NO_MEM;
        }
        memcpy(vm->name_constants, bc->name_constants, bc->name_constant_count * PLC_MAX_NAME_LEN);
        vm->name_constant_count = bc->name_constant_count;
    }

    vm->state = PLC_VM_STOPPED;
    vm->pc = 0;

    ESP_LOGI(TAG, "VM loaded: %" PRIu32 " instructions, %" PRIu32 " variables",
             vm->instruction_count, vm->variable_count);

    return ESP_OK;
}

void plc_vm_reset(plc_vm_t *vm) {
    vm->pc = 0;
    vm->stack_ptr = 0;
    vm->call_stack_ptr = 0;
    vm->cycle_count = 0;
    vm->scan_time_us = 0;
    vm->max_scan_time_us = 0;
    vm->total_scan_time_us = 0;
    vm->state = PLC_VM_STOPPED;
    vm->single_step = false;
    vm->error_msg[0] = '\0';

    // Reset timers
    for (int i = 0; i < PLC_VM_MAX_TIMERS; i++) {
        memset(&vm->timers[i], 0, sizeof(plc_timer_t));
    }

    // Reset counters
    for (int i = 0; i < PLC_VM_MAX_COUNTERS; i++) {
        memset(&vm->counters[i], 0, sizeof(plc_counter_t));
    }

    // Reset edge detection
    memset(vm->edge_prev, 0, sizeof(vm->edge_prev));

    // Clear breakpoints
    memset(vm->breakpoints, 0, sizeof(vm->breakpoints));
}

void plc_vm_cleanup(plc_vm_t *vm) {
    if (vm->instructions) {
        free(vm->instructions);
        vm->instructions = NULL;
    }
    if (vm->variables) {
        free(vm->variables);
        vm->variables = NULL;
    }
    if (vm->variable_names) {
        free(vm->variable_names);
        vm->variable_names = NULL;
    }
    if (vm->name_constants) {
        free(vm->name_constants);
        vm->name_constants = NULL;
    }
    vm->instruction_count = 0;
    vm->variable_count = 0;
    vm->name_constant_count = 0;
}

// --- Variable access ---

const plc_value_t *plc_vm_get_variable(const plc_vm_t *vm, uint32_t index) {
    if (!vm->variables || index >= vm->variable_count) return NULL;
    return &vm->variables[index];
}

const char *plc_vm_get_variable_name(const plc_vm_t *vm, uint32_t index) {
    if (!vm->variable_names || index >= vm->variable_count) return NULL;
    return vm->variable_names[index];
}

esp_err_t plc_vm_set_variable(plc_vm_t *vm, uint32_t index, const plc_value_t *val) {
    if (!vm->variables || index >= vm->variable_count) return ESP_ERR_INVALID_ARG;
    vm->variables[index] = *val;
    return ESP_OK;
}

esp_err_t plc_vm_force_variable(plc_vm_t *vm, uint32_t index, const plc_value_t *val) {
    if (!vm->variables || index >= vm->variable_count) return ESP_ERR_INVALID_ARG;
    vm->variables[index] = *val;
    vm->forced_vars[index / 32] |= (1U << (index % 32));
    return ESP_OK;
}

esp_err_t plc_vm_unforce_variable(plc_vm_t *vm, uint32_t index) {
    if (index >= vm->variable_count) return ESP_ERR_INVALID_ARG;
    vm->forced_vars[index / 32] &= ~(1U << (index % 32));
    return ESP_OK;
}

bool plc_vm_is_variable_forced(const plc_vm_t *vm, uint32_t index) {
    if (index >= vm->variable_count) return false;
    return (vm->forced_vars[index / 32] & (1U << (index % 32))) != 0;
}

// --- Breakpoints ---

void plc_vm_set_breakpoint(plc_vm_t *vm, uint32_t pc) {
    if (pc < PLC_MAX_INSTRUCTIONS) {
        vm->breakpoints[pc / 32] |= (1U << (pc % 32));
    }
}

void plc_vm_clear_breakpoint(plc_vm_t *vm, uint32_t pc) {
    if (pc < PLC_MAX_INSTRUCTIONS) {
        vm->breakpoints[pc / 32] &= ~(1U << (pc % 32));
    }
}

bool plc_vm_has_breakpoint(const plc_vm_t *vm, uint32_t pc) {
    if (pc >= PLC_MAX_INSTRUCTIONS) return false;
    return (vm->breakpoints[pc / 32] & (1U << (pc % 32))) != 0;
}

// --- I/O memory operations (for LoadIo/StoreIo) ---

// Input/Output/Memory image areas (simple flat arrays)
#define PLC_IO_IMAGE_SIZE  256
static plc_value_t s_input_image[PLC_IO_IMAGE_SIZE];
static plc_value_t s_output_image[PLC_IO_IMAGE_SIZE];
static plc_value_t s_memory_image[PLC_IO_IMAGE_SIZE];

static void read_io_image(uint32_t encoded, plc_vm_t *vm) {
    plc_value_t val = { .type = PLC_TYPE_DINT, .dint_val = 0 };
    if ((encoded >> 24) != 0) {
        uint8_t area = (encoded >> 28) & 0xF;
        uint32_t idx = encoded & 0x00FFFFFF;
        if (idx >= PLC_IO_IMAGE_SIZE) idx = 0;
        switch (area) {
            case 1: val = s_input_image[idx]; break;
            case 2: val = s_output_image[idx]; break;
            case 3: val = s_memory_image[idx]; break;
            default: break;
        }
    } else {
        if (encoded < PLC_IO_IMAGE_SIZE) {
            val = s_input_image[encoded];
        }
    }
    stack_push(vm, val);
}

static void write_io_image(uint32_t encoded, plc_value_t val) {
    if ((encoded >> 24) != 0) {
        uint8_t area = (encoded >> 28) & 0xF;
        uint32_t idx = encoded & 0x00FFFFFF;
        if (idx >= PLC_IO_IMAGE_SIZE) return;
        switch (area) {
            case 1: s_input_image[idx] = val; break;
            case 2: s_output_image[idx] = val; break;
            case 3: s_memory_image[idx] = val; break;
            default: break;
        }
    } else {
        if (encoded < PLC_IO_IMAGE_SIZE) {
            s_output_image[encoded] = val;
        }
    }
}

// --- Timer execution ---

static void execute_timer(plc_vm_t *vm, plc_timer_kind_t kind, int32_t operand) {
    int idx = operand;
    if (idx < 0 || idx >= PLC_VM_MAX_TIMERS) {
        ESP_LOGW(TAG, "timer index %d out of range", idx);
        plc_value_t zero_q = { .type = PLC_TYPE_BOOL, .bool_val = false };
        plc_value_t zero_et = { .type = PLC_TYPE_TIME, .time_val = 0 };
        stack_push(vm, zero_et);
        stack_push(vm, zero_q);
        return;
    }

    plc_value_t in_val, pt_val;
    if (!stack_pop(vm, &in_val) || !stack_pop(vm, &pt_val)) return;

    bool in = plc_value_to_bool(&in_val);
    int32_t pt = plc_value_to_int(&pt_val);

    plc_timer_t *t = &vm->timers[idx];
    t->kind = kind;
    t->pt = pt;

    int64_t now = esp_timer_get_time() / 1000; // ms

    switch (kind) {
        case PLC_TIMER_TON:
            if (in) {
                if (!t->timing) {
                    t->timing = true;
                    t->start_tick = now;
                }
                t->et = (int32_t)(now - t->start_tick);
                if (t->et >= pt) {
                    t->et = pt;
                    t->q = true;
                } else {
                    t->q = false;
                }
            } else {
                t->timing = false;
                t->q = false;
                t->et = 0;
            }
            break;

        case PLC_TIMER_TOF:
            if (in) {
                t->q = true;
                t->timing = false;
                t->et = 0;
            } else {
                if (t->in_prev && !in) {
                    // Falling edge: start timing
                    t->timing = true;
                    t->start_tick = now;
                }
                if (t->timing) {
                    t->et = (int32_t)(now - t->start_tick);
                    if (t->et >= pt) {
                        t->et = pt;
                        t->q = false;
                        t->timing = false;
                    }
                }
            }
            break;

        case PLC_TIMER_TP:
            if (in && !t->in_prev) {
                // Rising edge: start pulse
                t->timing = true;
                t->start_tick = now;
                t->q = true;
            }
            if (t->timing) {
                t->et = (int32_t)(now - t->start_tick);
                if (t->et >= pt) {
                    t->et = pt;
                    t->q = false;
                    t->timing = false;
                }
            }
            if (!in) {
                // Reset only after pulse completes
                if (!t->timing) {
                    t->et = 0;
                }
            }
            break;
    }

    t->in_prev = in;

    plc_value_t et_val = { .type = PLC_TYPE_TIME, .time_val = t->et };
    plc_value_t q_val = { .type = PLC_TYPE_BOOL, .bool_val = t->q };
    stack_push(vm, et_val);
    stack_push(vm, q_val);
}

// --- Counter execution ---

static void execute_ctu(plc_vm_t *vm, int32_t operand) {
    int idx = operand;
    plc_value_t cu_val, r_val, pv_val;
    if (!stack_pop(vm, &cu_val) || !stack_pop(vm, &r_val) || !stack_pop(vm, &pv_val)) return;

    bool cu = plc_value_to_bool(&cu_val);
    bool r = plc_value_to_bool(&r_val);
    int32_t pv = plc_value_to_int(&pv_val);

    if (idx < 0 || idx >= PLC_VM_MAX_COUNTERS) {
        plc_value_t zero = { .type = PLC_TYPE_DINT, .dint_val = 0 };
        plc_value_t q = { .type = PLC_TYPE_BOOL, .bool_val = false };
        stack_push(vm, zero);
        stack_push(vm, q);
        return;
    }

    plc_counter_t *c = &vm->counters[idx];

    if (r) {
        c->cv = 0;
        c->q = false;
    } else {
        // Rising edge detection on CU
        if (cu && !c->cu_prev) {
            c->cv++;
        }
        c->q = (c->cv >= pv);
    }
    c->cu_prev = cu;

    plc_value_t cv_out = { .type = PLC_TYPE_DINT, .dint_val = c->cv };
    plc_value_t q_out = { .type = PLC_TYPE_BOOL, .bool_val = c->q };
    stack_push(vm, cv_out);
    stack_push(vm, q_out);
}

static void execute_ctd(plc_vm_t *vm, int32_t operand) {
    int idx = operand;
    plc_value_t cd_val, ld_val, pv_val;
    if (!stack_pop(vm, &cd_val) || !stack_pop(vm, &ld_val) || !stack_pop(vm, &pv_val)) return;

    bool cd = plc_value_to_bool(&cd_val);
    bool ld = plc_value_to_bool(&ld_val);
    int32_t pv = plc_value_to_int(&pv_val);

    if (idx < 0 || idx >= PLC_VM_MAX_COUNTERS) {
        plc_value_t zero = { .type = PLC_TYPE_DINT, .dint_val = 0 };
        plc_value_t q = { .type = PLC_TYPE_BOOL, .bool_val = false };
        stack_push(vm, zero);
        stack_push(vm, q);
        return;
    }

    plc_counter_t *c = &vm->counters[idx];

    if (ld) {
        c->cv = pv;
        c->q = false;
    } else {
        if (cd && !c->cd_prev) {
            c->cv--;
        }
        c->q = (c->cv <= 0);
    }
    c->cd_prev = cd;

    plc_value_t cv_out = { .type = PLC_TYPE_DINT, .dint_val = c->cv };
    plc_value_t q_out = { .type = PLC_TYPE_BOOL, .bool_val = c->q };
    stack_push(vm, cv_out);
    stack_push(vm, q_out);
}

static void execute_ctud(plc_vm_t *vm, int32_t operand) {
    int idx = operand;
    plc_value_t cu_val, r_val, cd_val, ld_val, pv_val;
    if (!stack_pop(vm, &cu_val) || !stack_pop(vm, &r_val) ||
        !stack_pop(vm, &cd_val) || !stack_pop(vm, &ld_val) || !stack_pop(vm, &pv_val)) return;

    bool cu = plc_value_to_bool(&cu_val);
    bool r = plc_value_to_bool(&r_val);
    bool cd = plc_value_to_bool(&cd_val);
    bool ld = plc_value_to_bool(&ld_val);
    int32_t pv = plc_value_to_int(&pv_val);

    if (idx < 0 || idx >= PLC_VM_MAX_COUNTERS) {
        plc_value_t zero = { .type = PLC_TYPE_DINT, .dint_val = 0 };
        plc_value_t q = { .type = PLC_TYPE_BOOL, .bool_val = false };
        stack_push(vm, zero);
        stack_push(vm, q);
        return;
    }

    plc_counter_t *c = &vm->counters[idx];
    c->kind = PLC_COUNTER_CTUD;

    if (r) {
        c->cv = 0;
    } else if (ld) {
        c->cv = pv;
    } else {
        if (cu && !c->cu_prev) c->cv++;
        if (cd && !c->cd_prev) c->cv--;
    }
    c->q = (c->cv >= pv);
    c->cu_prev = cu;
    c->cd_prev = cd;

    plc_value_t cv_out = { .type = PLC_TYPE_DINT, .dint_val = c->cv };
    plc_value_t q_out = { .type = PLC_TYPE_BOOL, .bool_val = c->q };
    stack_push(vm, cv_out);
    stack_push(vm, q_out);
}

// --- Edge detection ---

static void execute_edge(plc_vm_t *vm, bool rising, int32_t operand) {
    int idx = operand;
    plc_value_t clk_val;
    if (!stack_pop(vm, &clk_val)) return;

    bool clk = plc_value_to_bool(&clk_val);

    if (idx < 0 || idx >= PLC_VM_MAX_EDGE_REGS) {
        plc_value_t q = { .type = PLC_TYPE_BOOL, .bool_val = false };
        stack_push(vm, q);
        return;
    }

    bool prev = vm->edge_prev[idx];
    vm->edge_prev[idx] = clk;

    bool q = rising ? (clk && !prev) : (!clk && prev);

    plc_value_t q_val = { .type = PLC_TYPE_BOOL, .bool_val = q };
    stack_push(vm, q_val);
}

// --- Type promotion helper ---

static plc_value_type_t promote_type(plc_value_type_t a, plc_value_type_t b) {
    if (a == PLC_TYPE_REAL || b == PLC_TYPE_REAL) return PLC_TYPE_REAL;
    if (a == PLC_TYPE_DINT || b == PLC_TYPE_DINT) return PLC_TYPE_DINT;
    if (a == PLC_TYPE_DWORD || b == PLC_TYPE_DWORD) return PLC_TYPE_DINT;
    if (a == PLC_TYPE_WORD || b == PLC_TYPE_WORD) return PLC_TYPE_INT;
    return PLC_TYPE_INT;
}

// --- Main execution loop ---

static uint32_t execute_instruction(plc_vm_t *vm, uint32_t pc) {
    if (pc >= vm->instruction_count) return vm->instruction_count;

    const plc_instruction_t *instr = &vm->instructions[pc];
    uint32_t next_pc = pc + 1;
    int32_t operand = instr->has_operand ? instr->operand : 0;

    plc_value_t a, b, result;
    float fa, fb;

    switch (instr->opcode) {
        // --- Push constants ---
        case OPC_PUSH_BOOL: {
            plc_value_t v = { .type = PLC_TYPE_BOOL, .bool_val = (operand != 0) };
            stack_push(vm, v);
            break;
        }
        case OPC_PUSH_INT: {
            plc_value_t v = { .type = PLC_TYPE_INT, .int_val = (int16_t)operand };
            stack_push(vm, v);
            break;
        }
        case OPC_PUSH_REAL: {
            plc_value_t v = { .type = PLC_TYPE_REAL };
            memcpy(&v.real_val, &operand, 4);
            stack_push(vm, v);
            break;
        }
        case OPC_PUSH_TIME: {
            plc_value_t v = { .type = PLC_TYPE_TIME, .time_val = operand };
            stack_push(vm, v);
            break;
        }
        case OPC_PUSH_DINT: {
            plc_value_t v = { .type = PLC_TYPE_DINT, .dint_val = operand };
            stack_push(vm, v);
            break;
        }
        case OPC_PUSH_ENUM: {
            plc_value_t v = { .type = PLC_TYPE_STRING, .str_idx = (uint16_t)operand };
            stack_push(vm, v);
            break;
        }
        case OPC_PUSH_STRING: {
            plc_value_t v = { .type = PLC_TYPE_STRING, .str_idx = (uint16_t)operand };
            stack_push(vm, v);
            break;
        }

        // --- Variable load/store ---
        case OPC_LOAD_VAR: {
            uint32_t slot = (uint32_t)operand;
            if (slot < vm->variable_count) {
                stack_push(vm, vm->variables[slot]);
            }
            break;
        }
        case OPC_STORE_VAR: {
            uint32_t slot = (uint32_t)operand;
            if (stack_pop(vm, &a) && slot < vm->variable_count) {
                if (!plc_vm_is_variable_forced(vm, slot)) {
                    vm->variables[slot] = a;
                }
            }
            break;
        }
        case OPC_LOAD_IO:
            read_io_image((uint32_t)operand, vm);
            break;
        case OPC_STORE_IO:
            if (stack_pop(vm, &a)) {
                write_io_image((uint32_t)operand, a);
            }
            break;

        // --- Arithmetic ---
        case OPC_ADD:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = promote_type(a.type, b.type);
                if (result.type == PLC_TYPE_REAL) {
                    result.real_val = plc_value_to_real(&a) + plc_value_to_real(&b);
                } else {
                    result.dint_val = plc_value_to_int(&a) + plc_value_to_int(&b);
                    if (result.type == PLC_TYPE_INT) result.int_val = (int16_t)result.dint_val;
                }
                stack_push(vm, result);
            }
            break;
        case OPC_SUB:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = promote_type(a.type, b.type);
                if (result.type == PLC_TYPE_REAL) {
                    result.real_val = plc_value_to_real(&a) - plc_value_to_real(&b);
                } else {
                    result.dint_val = plc_value_to_int(&a) - plc_value_to_int(&b);
                    if (result.type == PLC_TYPE_INT) result.int_val = (int16_t)result.dint_val;
                }
                stack_push(vm, result);
            }
            break;
        case OPC_MUL:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = promote_type(a.type, b.type);
                if (result.type == PLC_TYPE_REAL) {
                    result.real_val = plc_value_to_real(&a) * plc_value_to_real(&b);
                } else {
                    result.dint_val = plc_value_to_int(&a) * plc_value_to_int(&b);
                    if (result.type == PLC_TYPE_INT) result.int_val = (int16_t)result.dint_val;
                }
                stack_push(vm, result);
            }
            break;
        case OPC_DIV:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                fb = plc_value_to_real(&b);
                result.real_val = (fb == 0.0f) ? NAN : plc_value_to_real(&a) / fb;
                stack_push(vm, result);
            }
            break;
        case OPC_MODULO:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                fb = plc_value_to_real(&b);
                result.real_val = (fb == 0.0f) ? NAN : fmodf(plc_value_to_real(&a), fb);
                stack_push(vm, result);
            }
            break;
        case OPC_POW:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = powf(plc_value_to_real(&a), plc_value_to_real(&b));
                stack_push(vm, result);
            }
            break;
        case OPC_NEG:
            if (stack_pop(vm, &a)) {
                switch (a.type) {
                    case PLC_TYPE_INT:
                        a.int_val = -a.int_val;
                        break;
                    case PLC_TYPE_DINT:
                        a.dint_val = -a.dint_val;
                        break;
                    case PLC_TYPE_REAL:
                        a.real_val = -a.real_val;
                        break;
                    default:
                        break;
                }
                stack_push(vm, a);
            }
            break;

        // --- Comparison ---
        case OPC_EQ:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_BOOL;
                result.bool_val = (plc_value_to_real(&a) == plc_value_to_real(&b));
                stack_push(vm, result);
            }
            break;
        case OPC_NE:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_BOOL;
                result.bool_val = (plc_value_to_real(&a) != plc_value_to_real(&b));
                stack_push(vm, result);
            }
            break;
        case OPC_LT:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_BOOL;
                result.bool_val = (plc_value_to_real(&a) < plc_value_to_real(&b));
                stack_push(vm, result);
            }
            break;
        case OPC_LE:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_BOOL;
                result.bool_val = (plc_value_to_real(&a) <= plc_value_to_real(&b));
                stack_push(vm, result);
            }
            break;
        case OPC_GT:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_BOOL;
                result.bool_val = (plc_value_to_real(&a) > plc_value_to_real(&b));
                stack_push(vm, result);
            }
            break;
        case OPC_GE:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_BOOL;
                result.bool_val = (plc_value_to_real(&a) >= plc_value_to_real(&b));
                stack_push(vm, result);
            }
            break;

        // --- Logic/Bitwise ---
        case OPC_AND:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                if (a.type == PLC_TYPE_WORD || b.type == PLC_TYPE_WORD) {
                    result.type = PLC_TYPE_WORD;
                    result.word_val = (uint16_t)(plc_value_to_int(&a) & plc_value_to_int(&b));
                } else if (a.type == PLC_TYPE_DWORD || b.type == PLC_TYPE_DWORD) {
                    result.type = PLC_TYPE_DWORD;
                    result.dword_val = (uint32_t)plc_value_to_int(&a) & (uint32_t)plc_value_to_int(&b);
                } else {
                    result.type = PLC_TYPE_BOOL;
                    result.bool_val = plc_value_to_bool(&a) && plc_value_to_bool(&b);
                }
                stack_push(vm, result);
            }
            break;
        case OPC_OR:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                if (a.type == PLC_TYPE_WORD || b.type == PLC_TYPE_WORD) {
                    result.type = PLC_TYPE_WORD;
                    result.word_val = (uint16_t)(plc_value_to_int(&a) | plc_value_to_int(&b));
                } else if (a.type == PLC_TYPE_DWORD || b.type == PLC_TYPE_DWORD) {
                    result.type = PLC_TYPE_DWORD;
                    result.dword_val = (uint32_t)plc_value_to_int(&a) | (uint32_t)plc_value_to_int(&b);
                } else {
                    result.type = PLC_TYPE_BOOL;
                    result.bool_val = plc_value_to_bool(&a) || plc_value_to_bool(&b);
                }
                stack_push(vm, result);
            }
            break;
        case OPC_XOR:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                if (a.type == PLC_TYPE_WORD || b.type == PLC_TYPE_WORD) {
                    result.type = PLC_TYPE_WORD;
                    result.word_val = (uint16_t)(plc_value_to_int(&a) ^ plc_value_to_int(&b));
                } else if (a.type == PLC_TYPE_DWORD || b.type == PLC_TYPE_DWORD) {
                    result.type = PLC_TYPE_DWORD;
                    result.dword_val = (uint32_t)plc_value_to_int(&a) ^ (uint32_t)plc_value_to_int(&b);
                } else {
                    result.type = PLC_TYPE_BOOL;
                    bool ba = plc_value_to_bool(&a);
                    bool bb = plc_value_to_bool(&b);
                    result.bool_val = (ba && !bb) || (!ba && bb);
                }
                stack_push(vm, result);
            }
            break;
        case OPC_NOT:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_BOOL;
                result.bool_val = !plc_value_to_bool(&a);
                stack_push(vm, result);
            }
            break;
        case OPC_SHL:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                result.dint_val = (int32_t)((uint32_t)plc_value_to_int(&a) << plc_value_to_int(&b));
                stack_push(vm, result);
            }
            break;
        case OPC_SHR:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                result.dint_val = (int32_t)((uint32_t)plc_value_to_int(&a) >> plc_value_to_int(&b));
                stack_push(vm, result);
            }
            break;
        case OPC_ROL: {
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                uint32_t v = (uint32_t)plc_value_to_int(&a);
                uint32_t n = (uint32_t)plc_value_to_int(&b) % 32;
                uint32_t rotated = (n == 0) ? v : ((v << n) | (v >> (32 - n)));
                result.type = PLC_TYPE_DINT;
                result.dint_val = (int32_t)rotated;
                stack_push(vm, result);
            }
            break;
        }
        case OPC_ROR: {
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                uint32_t v = (uint32_t)plc_value_to_int(&a);
                uint32_t n = (uint32_t)plc_value_to_int(&b) % 32;
                uint32_t rotated = (n == 0) ? v : ((v >> n) | (v << (32 - n)));
                result.type = PLC_TYPE_DINT;
                result.dint_val = (int32_t)rotated;
                stack_push(vm, result);
            }
            break;
        }

        // --- Control flow ---
        case OPC_JUMP:
            next_pc = (uint32_t)((int32_t)pc + operand);
            break;
        case OPC_JUMP_IF_FALSE:
            if (stack_pop(vm, &a)) {
                if (!plc_value_to_bool(&a)) {
                    next_pc = (uint32_t)((int32_t)pc + operand);
                }
            }
            break;
        case OPC_CALL:
            // Legacy call (no-op in embedded context)
            break;
        case OPC_CALL_POU: {
            // POU call using name constant index
            // In embedded, we look up the entry point from a fixed POU table
            // For now, this is simplified -- the POU table would be embedded in bytecode
            // We push return address and jump
            uint32_t name_idx = (uint32_t)operand;
            // The operand points to the instruction index to jump to
            if (vm->call_stack_ptr < PLC_VM_CALL_STACK_SIZE) {
                vm->call_stack[vm->call_stack_ptr++] = pc + 1;
                next_pc = (uint32_t)operand;
            }
            break;
        }
        case OPC_RETURN:
            if (vm->call_stack_ptr > 0) {
                next_pc = vm->call_stack[--vm->call_stack_ptr];
            } else {
                next_pc = vm->instruction_count;
            }
            break;

        // --- Function blocks ---
        case OPC_TON_CALL:
            execute_timer(vm, PLC_TIMER_TON, operand);
            break;
        case OPC_TOF_CALL:
            execute_timer(vm, PLC_TIMER_TOF, operand);
            break;
        case OPC_TP_CALL:
            execute_timer(vm, PLC_TIMER_TP, operand);
            break;
        case OPC_CTU_CALL:
            execute_ctu(vm, operand);
            break;
        case OPC_CTD_CALL:
            execute_ctd(vm, operand);
            break;
        case OPC_CTUD_CALL:
            execute_ctud(vm, operand);
            break;
        case OPC_R_TRIG_CALL:
            execute_edge(vm, true, operand);
            break;
        case OPC_F_TRIG_CALL:
            execute_edge(vm, false, operand);
            break;

        // --- Math functions ---
        case OPC_SQRT:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = sqrtf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_LN:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = logf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_LOG:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = log10f(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_EXP:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = expf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_SIN:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = sinf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_COS:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = cosf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_TAN:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = tanf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_ASIN:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = asinf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_ACOS:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = acosf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_ATAN:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = atanf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_ABS:
            if (stack_pop(vm, &a)) {
                switch (a.type) {
                    case PLC_TYPE_INT:  a.int_val = abs(a.int_val); break;
                    case PLC_TYPE_DINT: a.dint_val = abs(a.dint_val); break;
                    case PLC_TYPE_REAL: a.real_val = fabsf(a.real_val); break;
                    default: break;
                }
                stack_push(vm, a);
            }
            break;
        case OPC_TRUNC:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                result.dint_val = (int32_t)truncf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_FLOOR:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                result.dint_val = (int32_t)floorf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_CEIL:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                result.dint_val = (int32_t)ceilf(plc_value_to_real(&a));
                stack_push(vm, result);
            }
            break;
        case OPC_EXPT:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = powf(plc_value_to_real(&a), plc_value_to_real(&b));
                stack_push(vm, result);
            }
            break;

        // --- String operations (simplified: strings stored as name constant indices) ---
        case OPC_STR_LEN: {
            // Pop string index, push length
            // For embedded, string length is stored in the name constant itself
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                if (vm->name_constants && a.str_idx < vm->name_constant_count) {
                    result.dint_val = (int32_t)strlen(vm->name_constants[a.str_idx]);
                } else {
                    result.dint_val = 0;
                }
                stack_push(vm, result);
            }
            break;
        }
        case OPC_STR_LEFT:
        case OPC_STR_RIGHT:
        case OPC_STR_MID:
        case OPC_CONCAT:
        case OPC_STR_INSERT:
        case OPC_STR_DELETE:
        case OPC_STR_REPLACE:
        case OPC_STR_FIND:
            // String operations are complex for embedded -- push a placeholder
            // Full string support requires dynamic memory which is avoided on ESP32
            // The VM will still execute, string ops produce empty results
            ESP_LOGW(TAG, "string op 0x%02X not fully supported on ESP32 at pc=%" PRIu32, instr->opcode, pc);
            {
                plc_value_t empty = { .type = PLC_TYPE_STRING, .str_idx = 0 };
                stack_push(vm, empty);
            }
            break;

        // --- Conversions ---
        case OPC_INT_TO_REAL:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                result.real_val = (float)plc_value_to_int(&a);
                stack_push(vm, result);
            }
            break;
        case OPC_REAL_TO_INT:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                result.dint_val = (int32_t)plc_value_to_real(&a);
                stack_push(vm, result);
            }
            break;
        case OPC_INT_TO_BOOL:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_BOOL;
                result.bool_val = (plc_value_to_int(&a) != 0);
                stack_push(vm, result);
            }
            break;
        case OPC_BOOL_TO_INT:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                result.dint_val = plc_value_to_bool(&a) ? 1 : 0;
                stack_push(vm, result);
            }
            break;
        case OPC_INT_TO_STRING:
        case OPC_REAL_TO_STRING:
            // Conversion to string stores result as a name constant
            // On ESP32, we generate a string into the name constants table
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_STRING;
                result.str_idx = 0; // placeholder
                stack_push(vm, result);
            }
            break;
        case OPC_STRING_TO_INT:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                if (vm->name_constants && a.str_idx < vm->name_constant_count) {
                    result.dint_val = atoi(vm->name_constants[a.str_idx]);
                } else {
                    result.dint_val = 0;
                }
                stack_push(vm, result);
            }
            break;
        case OPC_STRING_TO_REAL:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                if (vm->name_constants && a.str_idx < vm->name_constant_count) {
                    result.real_val = strtof(vm->name_constants[a.str_idx], NULL);
                } else {
                    result.real_val = 0.0f;
                }
                stack_push(vm, result);
            }
            break;
        case OPC_WORD_TO_INT:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                if (a.type == PLC_TYPE_WORD) {
                    result.dint_val = (int32_t)a.word_val;
                } else {
                    result.dint_val = plc_value_to_int(&a);
                }
                stack_push(vm, result);
            }
            break;
        case OPC_DWORD_TO_INT:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DINT;
                if (a.type == PLC_TYPE_DWORD) {
                    result.dint_val = (int32_t)a.dword_val;
                } else {
                    result.dint_val = plc_value_to_int(&a);
                }
                stack_push(vm, result);
            }
            break;
        case OPC_INT_TO_WORD:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_WORD;
                result.word_val = (uint16_t)plc_value_to_int(&a);
                stack_push(vm, result);
            }
            break;
        case OPC_INT_TO_DWORD:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DWORD;
                result.dword_val = (uint32_t)plc_value_to_int(&a);
                stack_push(vm, result);
            }
            break;
        case OPC_DWORD_TO_REAL:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_REAL;
                uint32_t bits;
                if (a.type == PLC_TYPE_DWORD) {
                    bits = a.dword_val;
                } else {
                    bits = (uint32_t)plc_value_to_int(&a);
                }
                memcpy(&result.real_val, &bits, 4);
                stack_push(vm, result);
            }
            break;
        case OPC_REAL_TO_DWORD:
            if (stack_pop(vm, &a)) {
                result.type = PLC_TYPE_DWORD;
                float f = plc_value_to_real(&a);
                memcpy(&result.dword_val, &f, 4);
                stack_push(vm, result);
            }
            break;

        // --- Selection ---
        case OPC_SEL: {
            plc_value_t g, in0, in1;
            if (stack_pop(vm, &g) && stack_pop(vm, &in1) && stack_pop(vm, &in0)) {
                stack_push(vm, plc_value_to_bool(&g) ? in1 : in0);
            }
            break;
        }
        case OPC_MUX: {
            plc_value_t k;
            if (stack_pop(vm, &k)) {
                int32_t idx = plc_value_to_int(&k);
                // The compiler pushes N inputs then K on top.
                // After popping K, the top N values on the stack are the inputs.
                // We need to pop all N inputs and push only the selected one.
                // N = vm->stack_ptr (remaining values are the inputs).
                // But we don't know N from the opcode alone.
                // Convention: operand contains the number of inputs N.
                int32_t n = (instr->has_operand) ? operand : 0;
                if (n <= 0) n = vm->stack_ptr;  // Fallback: treat all remaining as inputs

                if (idx >= 0 && idx < n && n <= (int32_t)vm->stack_ptr) {
                    // Inputs are at stack[stack_ptr - n] through stack[stack_ptr - 1]
                    plc_value_t val = vm->stack[vm->stack_ptr - n + idx];
                    vm->stack_ptr -= n;  // Pop all inputs
                    stack_push(vm, val);
                } else {
                    // Out of range: pop all inputs, push 0
                    vm->stack_ptr -= (n <= (int32_t)vm->stack_ptr) ? n : vm->stack_ptr;
                    result.type = PLC_TYPE_DINT;
                    result.dint_val = 0;
                    stack_push(vm, result);
                }
            }
            break;
        }
        case OPC_LIMIT: {
            plc_value_t max_v, min_v, val;
            if (stack_pop(vm, &max_v) && stack_pop(vm, &min_v) && stack_pop(vm, &val)) {
                float fv = plc_value_to_real(&val);
                float fmin = plc_value_to_real(&min_v);
                float fmax = plc_value_to_real(&max_v);
                if (fv < fmin) {
                    stack_push(vm, min_v);
                } else if (fv > fmax) {
                    stack_push(vm, max_v);
                } else {
                    stack_push(vm, val);
                }
            }
            break;
        }
        case OPC_MAX:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                fa = plc_value_to_real(&a);
                fb = plc_value_to_real(&b);
                result.type = PLC_TYPE_REAL;
                result.real_val = (fa > fb) ? fa : fb;
                stack_push(vm, result);
            }
            break;
        case OPC_MIN:
            if (stack_pop(vm, &b) && stack_pop(vm, &a)) {
                fa = plc_value_to_real(&a);
                fb = plc_value_to_real(&b);
                result.type = PLC_TYPE_REAL;
                result.real_val = (fa < fb) ? fa : fb;
                stack_push(vm, result);
            }
            break;
        case OPC_MOVE:
            // Move is a no-op: value is already on stack
            break;

        // --- Array ops (simplified for embedded) ---
        case OPC_LOAD_ARRAY_INDEX:
        case OPC_STORE_ARRAY_INDEX:
        case OPC_ARRAY_LEN:
            // Arrays require dynamic allocation -- simplified on ESP32
            ESP_LOGW(TAG, "array op 0x%02X not fully supported on ESP32 at pc=%" PRIu32, instr->opcode, pc);
            {
                result.type = PLC_TYPE_DINT;
                result.dint_val = 0;
                stack_push(vm, result);
            }
            break;

        // --- Struct ops (simplified for embedded) ---
        case OPC_LOAD_STRUCT_FIELD:
        case OPC_STORE_STRUCT_FIELD:
            ESP_LOGW(TAG, "struct op 0x%02X not fully supported on ESP32 at pc=%" PRIu32, instr->opcode, pc);
            {
                result.type = PLC_TYPE_DINT;
                result.dint_val = 0;
                stack_push(vm, result);
            }
            break;

        // --- Halt ---
        case OPC_HALT:
            return vm->instruction_count;

        default:
            ESP_LOGW(TAG, "unknown opcode 0x%02X at pc=%" PRIu32, instr->opcode, pc);
            break;
    }

    return next_pc;
}

esp_err_t plc_vm_execute_cycle(plc_vm_t *vm) {
    if (!vm->instructions || vm->instruction_count == 0) {
        vm->state = PLC_VM_ERROR;
        snprintf(vm->error_msg, sizeof(vm->error_msg), "no program loaded");
        return ESP_ERR_INVALID_STATE;
    }

    int64_t start_us = esp_timer_get_time();

    // Reset program counter and stack for this scan cycle
    vm->pc = 0;
    vm->stack_ptr = 0;
    vm->call_stack_ptr = 0;

    vm->state = PLC_VM_RUNNING;

    // Execute all instructions
#define PLC_MAX_INSTRUCTIONS_PER_CYCLE 100000
    uint32_t instr_count = 0;
    while (vm->pc < vm->instruction_count) {
        // Check breakpoints
        if (plc_vm_has_breakpoint(vm, vm->pc)) {
            vm->state = PLC_VM_BREAKPOINT_HIT;
            return ESP_OK;
        }

        vm->pc = execute_instruction(vm, vm->pc);

        if (++instr_count > PLC_MAX_INSTRUCTIONS_PER_CYCLE) {
            vm->state = PLC_VM_ERROR;
            snprintf(vm->error_msg, sizeof(vm->error_msg), "cycle exceeded %u instructions", PLC_MAX_INSTRUCTIONS_PER_CYCLE);
            return ESP_ERR_INVALID_STATE;
        }

        // Single step mode
        if (vm->single_step && vm->pc < vm->instruction_count) {
            vm->single_step = false;
            vm->state = PLC_VM_PAUSED;
            return ESP_OK;
        }
    }

    // Cycle complete
    int64_t end_us = esp_timer_get_time();
    vm->scan_time_us = (uint32_t)(end_us - start_us);
    if (vm->scan_time_us > vm->max_scan_time_us) {
        vm->max_scan_time_us = vm->scan_time_us;
    }
    vm->total_scan_time_us += vm->scan_time_us;
    vm->cycle_count++;

    vm->state = PLC_VM_STOPPED;
    return ESP_OK;
}
