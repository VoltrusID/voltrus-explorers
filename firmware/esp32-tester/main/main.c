#include "esp_log.h"
#include "esp_system.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "wifi_station.h"
#include "protocol_launcher.h"

static const char *TAG = "main";

void app_main(void) {
    ESP_LOGI(TAG, "ESP32 Protocol Tester starting...");

    esp_err_t ret = wifi_station_init();
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "WiFi connection failed, restarting in 5s...");
        vTaskDelay(pdMS_TO_TICKS(5000));
        esp_restart();
    }

    ret = protocol_launcher_start();
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "protocol launch failed");
    }
}
