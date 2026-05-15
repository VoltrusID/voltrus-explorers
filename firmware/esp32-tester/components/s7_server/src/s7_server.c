#include "s7_server.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "s7_server";

esp_err_t s7_server_init(void) {
    ESP_LOGI(TAG, "S7 Server initialized (port=%d, rack=%d, slot=%d)",
             CONFIG_S7_SERVER_PORT, CONFIG_S7_RACK, CONFIG_S7_SLOT);
    return ESP_OK;
}

esp_err_t s7_server_start(void) {
    ESP_LOGI(TAG, "S7 Server started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
