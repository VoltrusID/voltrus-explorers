#include "dnp3_outstation.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "dnp3_outstation";

esp_err_t dnp3_outstation_init(void) {
    ESP_LOGI(TAG, "DNP3 Outstation initialized (port=%d, addr=%d)",
             CONFIG_DNP3_TCP_PORT, CONFIG_DNP3_OUTSTATION_ADDR);
    return ESP_OK;
}

esp_err_t dnp3_outstation_start(void) {
    ESP_LOGI(TAG, "DNP3 Outstation started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
