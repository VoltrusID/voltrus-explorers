#include "modbus_rtu_slave.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "modbus_rtu_slave";

esp_err_t modbus_rtu_slave_init(void) {
    ESP_LOGI(TAG, "Modbus RTU Slave initialized (baud=%d, de_pin=%d, tx=%d, rx=%d)",
             CONFIG_MODBUS_RTU_BAUD, CONFIG_MODBUS_RTU_DE_PIN,
             CONFIG_MODBUS_RTU_TX_PIN, CONFIG_MODBUS_RTU_RX_PIN);
    return ESP_OK;
}

esp_err_t modbus_rtu_slave_start(void) {
    ESP_LOGI(TAG, "Modbus RTU Slave started — not yet implemented");
    while (1) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    return ESP_OK;
}
