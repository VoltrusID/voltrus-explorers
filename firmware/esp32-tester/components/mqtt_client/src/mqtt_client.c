#include "mqtt_client.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "mqtt_client";

esp_err_t mqtt_client_init(void) {
    ESP_LOGI(TAG, "MQTT Client initialized (broker=%s)", CONFIG_MQTT_BROKER_URL);
    return ESP_OK;
}

esp_err_t mqtt_client_start(void) {
    ESP_LOGI(TAG, "MQTT Client started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
