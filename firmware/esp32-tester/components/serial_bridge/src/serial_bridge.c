#include "serial_bridge.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "driver/uart.h"
#include "driver/gpio.h"
#include <string.h>
#include <errno.h>

static const char *TAG = "serial_bridge";

#define UART0_NUM   UART_NUM_0
#define UART2_NUM   UART_NUM_2
#define BUF_SIZE    1024
#define RD_BUF_SIZE 1024
#define UART2_TX    CONFIG_SERIAL_BRIDGE_TX_PIN
#define UART2_RX    CONFIG_SERIAL_BRIDGE_RX_PIN
#define DE_PIN      CONFIG_SERIAL_BRIDGE_DE_PIN
#define BAUD_RATE   CONFIG_SERIAL_BRIDGE_BAUD

static bool running = false;
static volatile uint32_t bytes_pc_to_rs = 0;
static volatile uint32_t bytes_rs_to_pc = 0;

/* Forward bytes: UART0 RX -> UART2 TX (PC -> RS485) */
static void pc_to_rs485_task(void *arg) {
    uint8_t *data = malloc(RD_BUF_SIZE);
    if (!data) {
        ESP_LOGE(TAG, "pc_to_rs alloc failed");
        vTaskDelete(NULL);
        return;
    }

    while (running) {
        int len = uart_read_bytes(UART0_NUM, data, RD_BUF_SIZE, pdMS_TO_TICKS(100));
        if (len > 0) {
            /* Assert DE for RS485 transmit */
            gpio_set_level(DE_PIN, 1);
            uart_write_bytes(UART2_NUM, data, len);
            uart_wait_tx_done(UART2_NUM, pdMS_TO_TICKS(100));
            /* De-assert DE after transmit */
            gpio_set_level(DE_PIN, 0);

            bytes_pc_to_rs += len;
            if (bytes_pc_to_rs % 100 < (uint32_t)len) {
                ESP_LOGI(TAG, "PC->RS485: %lu bytes total", (unsigned long)bytes_pc_to_rs);
            }
        }
    }

    free(data);
    vTaskDelete(NULL);
}

/* Forward bytes: UART2 RX -> UART0 TX (RS485 -> PC) */
static void rs485_to_pc_task(void *arg) {
    uint8_t *data = malloc(RD_BUF_SIZE);
    if (!data) {
        ESP_LOGE(TAG, "rs_to_pc alloc failed");
        vTaskDelete(NULL);
        return;
    }

    while (running) {
        int len = uart_read_bytes(UART2_NUM, data, RD_BUF_SIZE, pdMS_TO_TICKS(100));
        if (len > 0) {
            uart_write_bytes(UART0_NUM, data, len);

            bytes_rs_to_pc += len;
            if (bytes_rs_to_pc % 100 < (uint32_t)len) {
                ESP_LOGI(TAG, "RS485->PC: %lu bytes total", (unsigned long)bytes_rs_to_pc);
            }
        }
    }

    free(data);
    vTaskDelete(NULL);
}

esp_err_t serial_bridge_init(void) {
    /* Configure UART2 for RS485 */
    uart_config_t uart2_config = {
        .baud_rate = BAUD_RATE,
        .data_bits = UART_DATA_8_BITS,
        .parity = UART_PARITY_DISABLE,
        .stop_bits = UART_STOP_BITS_1,
        .flow_ctrl = UART_HW_FLOWCTRL_DISABLE,
        .rx_flow_ctrl_thresh = 122,
        .source_clk = UART_SCLK_DEFAULT,
    };

    esp_err_t ret = uart_param_config(UART2_NUM, &uart2_config);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "UART2 param config failed: %s", esp_err_to_name(ret));
        return ret;
    }

    ret = uart_set_pin(UART2_NUM, UART2_TX, UART2_RX, UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "UART2 set pin failed: %s", esp_err_to_name(ret));
        return ret;
    }

    /* Install UART driver for UART2 with event queue */
    QueueHandle_t uart2_queue;
    ret = uart_driver_install(UART2_NUM, 2048, 0, 20, &uart2_queue, 0);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "UART2 driver install failed: %s", esp_err_to_name(ret));
        return ret;
    }

    /* Configure DE pin as output for RS485 direction control */
    gpio_set_direction(DE_PIN, GPIO_MODE_OUTPUT);
    gpio_set_level(DE_PIN, 0); /* Default: receive mode */

    /* Install UART driver for UART0 if not already installed */
    if (!uart_is_driver_installed(UART0_NUM)) {
        ret = uart_driver_install(UART0_NUM, 2048, 0, 0, NULL, 0);
        if (ret != ESP_OK) {
            ESP_LOGE(TAG, "UART0 driver install failed: %s", esp_err_to_name(ret));
            return ret;
        }
    }

    running = true;

    ESP_LOGI(TAG, "Serial Bridge initialized (baud=%d, tx=%d, rx=%d, de=%d)",
             BAUD_RATE, UART2_TX, UART2_RX, DE_PIN);
    return ESP_OK;
}

esp_err_t serial_bridge_start(void) {
    xTaskCreate(pc_to_rs485_task, "pc_to_rs", 4096, NULL,
                tskIDLE_PRIORITY + 5, NULL);
    xTaskCreate(rs485_to_pc_task, "rs_to_pc", 4096, NULL,
                tskIDLE_PRIORITY + 5, NULL);
    return ESP_OK;
}
