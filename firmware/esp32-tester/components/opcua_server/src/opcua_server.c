#include "opcua_server.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "opcua_server";

esp_err_t opcua_server_init(void) {
    ESP_LOGI(TAG, "OPC UA Server initialized (port=%d)", CONFIG_OPCUA_SERVER_PORT);
    return ESP_OK;
}

esp_err_t opcua_server_start(void) {
    ESP_LOGI(TAG, "OPC UA Server started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
