#include "plc_runtime.h"
#include "plc_ws_server.h"
#include "plc_io_bridge.h"
#include "plc_bytecode.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/semphr.h"
#include <string.h>
#include <inttypes.h>

static const char *TAG = "plc_runtime";

// Scan cycle task configuration
#define PLC_SCAN_TASK_STACK_SIZE  4096
#define PLC_SCAN_TASK_PRIORITY    5
#define PLC_SCAN_TASK_CORE        1  // Run on Core 1

// Runtime state (singleton)
static struct {
    plc_vm_t       vm;
    TaskHandle_t   scan_task;
    volatile bool  running;
    uint32_t       cycle_time_ms;
    plc_state_t    state;
    bool           initialized;
    SemaphoreHandle_t mutex;
} s_runtime;

// Forward declaration
static void plc_scan_task(void *arg);

esp_err_t plc_runtime_init(void) {
    if (s_runtime.initialized) {
        ESP_LOGW(TAG, "runtime already initialized");
        return ESP_OK;
    }

    memset(&s_runtime, 0, sizeof(s_runtime));
    plc_vm_init(&s_runtime.vm);

    s_runtime.mutex = xSemaphoreCreateRecursiveMutex();
    if (!s_runtime.mutex) {
        ESP_LOGE(TAG, "failed to create runtime mutex");
        return ESP_ERR_NO_MEM;
    }

    // Initialize I/O bridge
    esp_err_t ret = plc_io_bridge_init();
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "failed to init I/O bridge");
        return ret;
    }

    s_runtime.state = PLC_STATE_STOPPED;
    s_runtime.initialized = true;

    ESP_LOGI(TAG, "PLC runtime initialized");
    return ESP_OK;
}

esp_err_t plc_runtime_load_bytecode(const uint8_t *data, uint32_t len) {
    if (!s_runtime.initialized) {
        ESP_LOGE(TAG, "runtime not initialized");
        return ESP_ERR_INVALID_STATE;
    }

    // Stop any running scan first
    if (s_runtime.running) {
        plc_runtime_stop();
    }

    xSemaphoreTakeRecursive(s_runtime.mutex, portMAX_DELAY);

    // Parse and validate bytecode
    plc_bytecode_t bc = {0};
    esp_err_t ret = plc_bytecode_load(data, len, &bc);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "bytecode load failed: %s", esp_err_to_name(ret));
        xSemaphoreGiveRecursive(s_runtime.mutex);
        return ret;
    }

    // Configure I/O bridge with mappings from bytecode (before freeing bc)
    if (bc.io_mapping_count > 0 && bc.io_mappings) {
        plc_io_bridge_configure(bc.io_mappings, bc.io_mapping_count);
    }

    // Load into VM
    ret = plc_vm_load_bytecode(&s_runtime.vm, &bc);
    plc_bytecode_free(&bc);

    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "VM load failed: %s", esp_err_to_name(ret));
        xSemaphoreGiveRecursive(s_runtime.mutex);
        return ret;
    }

    s_runtime.state = PLC_STATE_STOPPED;
    xSemaphoreGiveRecursive(s_runtime.mutex);

    ESP_LOGI(TAG, "bytecode loaded, runtime ready");

    return ESP_OK;
}

esp_err_t plc_runtime_start(uint32_t cycle_time_ms) {
    if (!s_runtime.initialized) {
        return ESP_ERR_INVALID_STATE;
    }

    if (s_runtime.running) {
        ESP_LOGW(TAG, "runtime already running");
        return ESP_OK;
    }

    if (s_runtime.vm.instruction_count == 0) {
        ESP_LOGE(TAG, "no program loaded");
        return ESP_ERR_INVALID_STATE;
    }

    if (cycle_time_ms < 1) cycle_time_ms = 1;
    if (cycle_time_ms > 10000) cycle_time_ms = 10000;

    s_runtime.cycle_time_ms = cycle_time_ms;
    s_runtime.running = true;
    s_runtime.state = PLC_STATE_RUNNING;

    BaseType_t xResult = xTaskCreatePinnedToCore(
        plc_scan_task,
        "plc_scan",
        PLC_SCAN_TASK_STACK_SIZE,
        NULL,
        PLC_SCAN_TASK_PRIORITY,
        &s_runtime.scan_task,
        PLC_SCAN_TASK_CORE
    );

    if (xResult != pdPASS) {
        ESP_LOGE(TAG, "failed to create scan task");
        s_runtime.running = false;
        s_runtime.state = PLC_STATE_ERROR;
        return ESP_FAIL;
    }

    ESP_LOGI(TAG, "scan task started, cycle=%" PRIu32 "ms", cycle_time_ms);
    return ESP_OK;
}

esp_err_t plc_runtime_stop(void) {
    if (!s_runtime.running) {
        return ESP_OK;
    }

    TaskHandle_t task = s_runtime.scan_task;
    s_runtime.running = false;
    s_runtime.state = PLC_STATE_STOPPED;

    // Wait for task to signal completion
    if (task) {
        // Wait up to cycle_time + 500ms for the task to exit
        uint32_t wait_ms = s_runtime.cycle_time_ms + 500;
        vTaskDelay(pdMS_TO_TICKS(wait_ms));
        s_runtime.scan_task = NULL;
    }

    ESP_LOGI(TAG, "scan task stopped, cycles=%" PRIu32, s_runtime.vm.cycle_count);
    return ESP_OK;
}

esp_err_t plc_runtime_step(void) {
    if (!s_runtime.initialized) {
        return ESP_ERR_INVALID_STATE;
    }

    if (s_runtime.running) {
        ESP_LOGW(TAG, "cannot step while running");
        return ESP_ERR_INVALID_STATE;
    }

    if (s_runtime.vm.instruction_count == 0) {
        ESP_LOGE(TAG, "no program loaded");
        return ESP_ERR_INVALID_STATE;
    }

    // Read inputs
    xSemaphoreTakeRecursive(s_runtime.mutex, portMAX_DELAY);
    plc_io_bridge_read_inputs(&s_runtime.vm);

    // Execute one cycle
    esp_err_t ret = plc_vm_execute_cycle(&s_runtime.vm);

    // Write outputs
    plc_io_bridge_write_outputs(&s_runtime.vm);
    xSemaphoreGiveRecursive(s_runtime.mutex);

    // Push variable snapshot to WebSocket clients
    plc_ws_push_variables(&s_runtime.vm);

    return ret;
}

plc_runtime_status_t plc_runtime_get_status(void) {
    plc_runtime_status_t status = {0};
    xSemaphoreTakeRecursive(s_runtime.mutex, portMAX_DELAY);
    status.state = s_runtime.state;
    status.cycle_count = s_runtime.vm.cycle_count;
    status.scan_time_us = s_runtime.vm.scan_time_us;
    status.max_scan_time_us = s_runtime.vm.max_scan_time_us;
    status.variable_count = s_runtime.vm.variable_count;

    if (s_runtime.vm.cycle_count > 0) {
        status.avg_scan_time_us = s_runtime.vm.total_scan_time_us / s_runtime.vm.cycle_count;
    }
    xSemaphoreGiveRecursive(s_runtime.mutex);

    return status;
}

plc_vm_t *plc_runtime_get_vm(void) {
    return &s_runtime.vm;
}

plc_state_t plc_runtime_get_state(void) {
    return s_runtime.state;
}

uint32_t plc_runtime_get_cycle_count(void) {
    return s_runtime.vm.cycle_count;
}

uint32_t plc_runtime_get_scan_time_us(void) {
    return s_runtime.vm.scan_time_us;
}

esp_err_t plc_runtime_force_variable(uint32_t var_index, const plc_value_t *val) {
    xSemaphoreTakeRecursive(s_runtime.mutex, portMAX_DELAY);
    esp_err_t ret = plc_vm_force_variable(&s_runtime.vm, var_index, val);
    xSemaphoreGiveRecursive(s_runtime.mutex);
    return ret;
}

esp_err_t plc_runtime_force_variable_by_name(const char *name, const plc_value_t *val) {
    xSemaphoreTakeRecursive(s_runtime.mutex, portMAX_DELAY);
    for (uint32_t i = 0; i < s_runtime.vm.variable_count; i++) {
        const char *vname = plc_vm_get_variable_name(&s_runtime.vm, i);
        if (vname && strcmp(vname, name) == 0) {
            esp_err_t ret = plc_vm_force_variable(&s_runtime.vm, i, val);
            xSemaphoreGiveRecursive(s_runtime.mutex);
            return ret;
        }
    }
    xSemaphoreGiveRecursive(s_runtime.mutex);
    return ESP_ERR_NOT_FOUND;
}

esp_err_t plc_runtime_unforce_variable_by_name(const char *name) {
    xSemaphoreTakeRecursive(s_runtime.mutex, portMAX_DELAY);
    for (uint32_t i = 0; i < s_runtime.vm.variable_count; i++) {
        const char *vname = plc_vm_get_variable_name(&s_runtime.vm, i);
        if (vname && strcmp(vname, name) == 0) {
            esp_err_t ret = plc_runtime_unforce_variable(i);
            xSemaphoreGiveRecursive(s_runtime.mutex);
            return ret;
        }
    }
    xSemaphoreGiveRecursive(s_runtime.mutex);
    return ESP_ERR_NOT_FOUND;
}

esp_err_t plc_runtime_unforce_variable(uint32_t var_index) {
    // For now, unforce is a no-op (no forced flag tracking)
    // In the future, we'd clear a forced flag and let the VM overwrite naturally
    (void)var_index;
    return ESP_OK;
}

int plc_runtime_get_variables_json(char *buf, int buf_len) {
    xSemaphoreTakeRecursive(s_runtime.mutex, portMAX_DELAY);

    int pos = 0;
    buf[pos++] = '{';

    plc_vm_t *vm = &s_runtime.vm;
    for (uint32_t i = 0; i < vm->variable_count; i++) {
        if (i > 0) buf[pos++] = ',';

        const char *name = plc_vm_get_variable_name(vm, i);
        plc_value_t *val = &vm->variables[i];

        // Write "name":
        pos += snprintf(buf + pos, buf_len - pos, "\"%s\":", name ? name : "");

        // Convert value to number based on type
        double num_val = 0;
        switch (val->type) {
            case PLC_TYPE_BOOL:   num_val = val->bool_val ? 1.0 : 0.0; break;
            case PLC_TYPE_BYTE:   num_val = val->byte_val; break;
            case PLC_TYPE_INT:    num_val = val->int_val; break;
            case PLC_TYPE_DINT:   num_val = val->dint_val; break;
            case PLC_TYPE_REAL:   num_val = val->real_val; break;
            case PLC_TYPE_TIME:   num_val = val->time_val; break;
            case PLC_TYPE_WORD:   num_val = val->word_val; break;
            case PLC_TYPE_DWORD:  num_val = val->dword_val; break;
            case PLC_TYPE_STRING: num_val = val->str_idx; break;
            default: break;
        }

        if (val->type == PLC_TYPE_REAL) {
            pos += snprintf(buf + pos, buf_len - pos, "%.6g", num_val);
        } else {
            pos += snprintf(buf + pos, buf_len - pos, "%.0f", num_val);
        }

        if (pos >= buf_len - 32) break; // prevent overflow
    }

    buf[pos++] = '}';
    buf[pos] = '\0';

    xSemaphoreGiveRecursive(s_runtime.mutex);
    return pos;
}

// --- Scan cycle task (runs on Core 1) ---

static void plc_scan_task(void *arg) {
    ESP_LOGI(TAG, "scan task running on core %d", xPortGetCoreID());

    TickType_t last_wake = xTaskGetTickCount();
    const TickType_t cycle_ticks = pdMS_TO_TICKS(s_runtime.cycle_time_ms);

    while (s_runtime.running) {
        // Read inputs from protocol handlers / GPIO
        xSemaphoreTakeRecursive(s_runtime.mutex, portMAX_DELAY);
        plc_io_bridge_read_inputs(&s_runtime.vm);

        // Execute one scan cycle
        esp_err_t ret = plc_vm_execute_cycle(&s_runtime.vm);
        if (ret != ESP_OK) {
            ESP_LOGE(TAG, "VM error at cycle %" PRIu32, s_runtime.vm.cycle_count);
            s_runtime.state = PLC_STATE_ERROR;
            s_runtime.running = false;
            xSemaphoreGiveRecursive(s_runtime.mutex);
            break;
        }

        // Check for breakpoint or pause
        if (s_runtime.vm.state == PLC_VM_BREAKPOINT_HIT || s_runtime.vm.state == PLC_VM_PAUSED) {
            s_runtime.state = PLC_STATE_PAUSED;
            xSemaphoreGiveRecursive(s_runtime.mutex);
            // Push current state and wait for resume
            plc_ws_push_variables(&s_runtime.vm);
            while (s_runtime.running &&
                   s_runtime.vm.state == PLC_VM_BREAKPOINT_HIT) {
                vTaskDelay(pdMS_TO_TICKS(10));
            }
            if (!s_runtime.running) break;
            s_runtime.state = PLC_STATE_RUNNING;
        } else {
            // Write outputs to protocol handlers / GPIO
            plc_io_bridge_write_outputs(&s_runtime.vm);
            xSemaphoreGiveRecursive(s_runtime.mutex);
        }

        // Stream variables to WebSocket clients
        plc_ws_push_variables(&s_runtime.vm);

        // Wait for next cycle
        vTaskDelayUntil(&last_wake, cycle_ticks);
    }

    ESP_LOGI(TAG, "scan task exiting");
    s_runtime.scan_task = NULL;
    vTaskDelete(NULL);
}
