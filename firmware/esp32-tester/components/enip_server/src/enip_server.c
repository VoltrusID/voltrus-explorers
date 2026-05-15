#include "enip_server.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "enip_server";

esp_err_t enip_server_init(void) {
    ESP_LOGI(TAG, "EtherNet/IP Server initialized (port=%d)", CONFIG_ENIP_SERVER_PORT);
    return ESP_OK;
}

esp_err_t enip_server_start(void) {
    ESP_LOGI(TAG, "EtherNet/IP Server started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
