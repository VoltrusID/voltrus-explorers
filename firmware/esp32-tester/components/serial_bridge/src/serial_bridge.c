#include "serial_bridge.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "serial_bridge";

esp_err_t serial_bridge_init(void) {
    ESP_LOGI(TAG, "Serial Bridge initialized (baud=%d)", CONFIG_SERIAL_BRIDGE_BAUD);
    return ESP_OK;
}

esp_err_t serial_bridge_start(void) {
    ESP_LOGI(TAG, "Serial Bridge started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
