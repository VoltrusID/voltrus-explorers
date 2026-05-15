#include "bacnet_device.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "bacnet_device";

esp_err_t bacnet_device_init(void) {
    ESP_LOGI(TAG, "BACnet Device initialized (port=%d, device_id=%d)",
             CONFIG_BACNET_UDP_PORT, CONFIG_BACNET_DEVICE_ID);
    return ESP_OK;
}

esp_err_t bacnet_device_start(void) {
    ESP_LOGI(TAG, "BACnet Device started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
