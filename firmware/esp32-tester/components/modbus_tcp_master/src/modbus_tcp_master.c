#include "modbus_tcp_master.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "modbus_tcp_master";

esp_err_t modbus_tcp_master_init(void) {
    ESP_LOGI(TAG, "Modbus TCP Master initialized");
    return ESP_OK;
}

esp_err_t modbus_tcp_master_start(void) {
    ESP_LOGI(TAG, "Modbus TCP Master started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
