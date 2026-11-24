#include "modbus_tcp_master.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "mbcontroller.h"
#include <string.h>

static const char *TAG = "modbus_tcp_master";

#define SERVER_IP   CONFIG_MODBUS_MASTER_SERVER_IP
#define SERVER_PORT CONFIG_MODBUS_MASTER_SERVER_PORT
#define POLL_MS     CONFIG_MODBUS_MASTER_POLL_INTERVAL_MS
#define SLAVE_ADDR  1

static void *master_handler = NULL;
static bool running = false;

esp_err_t modbus_tcp_master_init(esp_netif_t *netif) {
    esp_err_t err = mbc_master_init_tcp(&master_handler);
    if (err != ESP_OK || master_handler == NULL) {
        ESP_LOGE(TAG, "mbc_master_init_tcp failed: 0x%x", err);
        return err;
    }

    mb_communication_info_t comm_info = {0};
    comm_info.ip_mode = MB_MODE_TCP;
    comm_info.ip_port = SERVER_PORT;
    comm_info.ip_addr_type = MB_IPV4;
    comm_info.ip_addr = NULL;
    comm_info.ip_netif_ptr = (void *)netif;

    err = mbc_master_setup((void *)&comm_info);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "mbc_master_setup failed: 0x%x", err);
        return err;
    }

    running = true;
    ESP_LOGI(TAG, "Modbus TCP Master init (target=%s:%d, poll=%dms)",
             SERVER_IP, SERVER_PORT, POLL_MS);
    return ESP_OK;
}

static void master_poll_task(void *arg) {
    ESP_ERROR_CHECK(mbc_master_start());

    bool coil_state = false;
    uint16_t regs[10];

    while (running) {
        // Read 10 holding registers from slave
        mb_param_request_t read_req = {
            .slave_addr = SLAVE_ADDR,
            .command = 0x03,
            .reg_start = 0,
            .reg_size = 10,
        };
        esp_err_t err = mbc_master_send_request(&read_req, regs);
        if (err == ESP_OK) {
            ESP_LOGI(TAG, "Read regs 0-9: %u %u %u %u %u %u %u %u %u %u",
                     regs[0], regs[1], regs[2], regs[3], regs[4],
                     regs[5], regs[6], regs[7], regs[8], regs[9]);
        } else {
            ESP_LOGW(TAG, "Read failed: 0x%x", err);
        }

        // Toggle coil 0
        coil_state = !coil_state;
        mb_param_request_t coil_req = {
            .slave_addr = SLAVE_ADDR,
            .command = 0x05,
            .reg_start = 0,
            .reg_size = 1,
        };
        uint16_t coil_val = coil_state ? 0xFF00 : 0x0000;
        err = mbc_master_send_request(&coil_req, &coil_val);
        if (err == ESP_OK) {
            ESP_LOGI(TAG, "Coil 0 -> %s", coil_state ? "ON" : "OFF");
        }

        vTaskDelay(pdMS_TO_TICKS(POLL_MS));
    }
    vTaskDelete(NULL);
}

esp_err_t modbus_tcp_master_start(void) {
    xTaskCreate(master_poll_task, "mb_master", 4096, NULL, tskIDLE_PRIORITY + 5, NULL);
    return ESP_OK;
}
