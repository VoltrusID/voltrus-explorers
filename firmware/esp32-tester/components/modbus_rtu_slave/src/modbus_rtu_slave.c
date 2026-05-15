#include "modbus_rtu_slave.h"

#include <errno.h>
#include <string.h>
#include <stdlib.h>

#include "esp_log.h"
#include "esp_system.h"
#include "esp_random.h"
#include "driver/uart.h"
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/queue.h"

static const char *TAG = "modbus_rtu_slave";

/* ---------- Configuration ---------- */

#define UART_NUM         UART_NUM_2
#define UART_BUF_SIZE    256
#define UART_READ_TIMEOUT_MS  100

#define SLAVE_ID         CONFIG_MODBUS_RTU_SLAVE_ID
#define BAUD_RATE        CONFIG_MODBUS_RTU_BAUD
#define DE_PIN           CONFIG_MODBUS_RTU_DE_PIN
#define TX_PIN           CONFIG_MODBUS_RTU_TX_PIN
#define RX_PIN           CONFIG_MODBUS_RTU_RX_PIN
#define LED_PIN          2

#define REG_COUNT        100
#define COIL_COUNT       100

/* ---------- Data model ---------- */

static uint16_t holding_regs[REG_COUNT];
static uint16_t input_regs[REG_COUNT];
static uint8_t  coils[(COIL_COUNT + 7) / 8];
static uint8_t  discrete_inputs[(COIL_COUNT + 7) / 8];

static QueueHandle_t uart_queue;

/* ---------- CRC16 (Modbus) ---------- */

static uint16_t crc16(const uint8_t *data, size_t len)
{
    uint16_t crc = 0xFFFF;
    for (size_t i = 0; i < len; i++) {
        crc ^= (uint16_t)data[i];
        for (int j = 0; j < 8; j++) {
            if (crc & 1) {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    return crc;
}

/* ---------- Coil / bit helpers ---------- */

static bool coil_get(uint16_t addr)
{
    if (addr >= COIL_COUNT) return false;
    return (coils[addr / 8] >> (addr % 8)) & 1;
}

static void coil_set(uint16_t addr, bool val)
{
    if (addr >= COIL_COUNT) return;
    if (val) {
        coils[addr / 8] |= (1 << (addr % 8));
    } else {
        coils[addr / 8] &= ~(1 << (addr % 8));
    }
}

static bool discrete_get(uint16_t addr)
{
    if (addr >= COIL_COUNT) return false;
    return (discrete_inputs[addr / 8] >> (addr % 8)) & 1;
}

/* ---------- LED control ---------- */

static void update_led(void)
{
    gpio_set_level(LED_PIN, coil_get(0) ? 1 : 0);
}

/* ---------- RS485 DE control ---------- */

static void rs485_tx_mode(void)
{
    gpio_set_level(DE_PIN, 1);
}

static void rs485_rx_mode(void)
{
    /* Wait for TX to complete before switching to RX */
    uart_wait_tx_done(UART_NUM, pdMS_TO_TICKS(100));
    gpio_set_level(DE_PIN, 0);
}

/* ---------- Response builder helpers ---------- */

static void send_response(const uint8_t *data, size_t len)
{
    rs485_tx_mode();
    uart_write_bytes(UART_NUM, data, len);
    rs485_rx_mode();
}

static void send_exception(uint8_t slave, uint8_t fc, uint8_t code)
{
    uint8_t resp[5];
    resp[0] = slave;
    resp[1] = fc | 0x80;
    resp[2] = code;
    uint16_t crc = crc16(resp, 3);
    resp[3] = crc & 0xFF;
    resp[4] = (crc >> 8) & 0xFF;
    send_response(resp, 5);
    ESP_LOGW(TAG, "Exception: FC=0x%02X, code=0x%02X", fc, code);
}

/* ---------- Function code handlers ---------- */

/* FC01 - Read Coils */
static void handle_fc01(uint8_t slave, const uint8_t *frame, uint16_t start, uint16_t qty)
{
    if (qty < 1 || qty > 2000 || (start + qty) > COIL_COUNT) {
        send_exception(slave, 0x01, 0x02); /* Illegal Data Address */
        return;
    }
    uint8_t byte_count = (qty + 7) / 8;
    uint8_t resp[256];
    size_t idx = 0;
    resp[idx++] = slave;
    resp[idx++] = 0x01;
    resp[idx++] = byte_count;
    memset(&resp[idx], 0, byte_count);
    for (uint16_t i = 0; i < qty; i++) {
        if (coil_get(start + i)) {
            resp[idx + i / 8] |= (1 << (i % 8));
        }
    }
    idx += byte_count;
    uint16_t crc = crc16(resp, idx);
    resp[idx++] = crc & 0xFF;
    resp[idx++] = (crc >> 8) & 0xFF;
    send_response(resp, idx);
    (void)frame;
}

/* FC02 - Read Discrete Inputs */
static void handle_fc02(uint8_t slave, const uint8_t *frame, uint16_t start, uint16_t qty)
{
    if (qty < 1 || qty > 2000 || (start + qty) > COIL_COUNT) {
        send_exception(slave, 0x02, 0x02);
        return;
    }
    uint8_t byte_count = (qty + 7) / 8;
    uint8_t resp[256];
    size_t idx = 0;
    resp[idx++] = slave;
    resp[idx++] = 0x02;
    resp[idx++] = byte_count;
    memset(&resp[idx], 0, byte_count);
    for (uint16_t i = 0; i < qty; i++) {
        if (discrete_get(start + i)) {
            resp[idx + i / 8] |= (1 << (i % 8));
        }
    }
    idx += byte_count;
    uint16_t crc = crc16(resp, idx);
    resp[idx++] = crc & 0xFF;
    resp[idx++] = (crc >> 8) & 0xFF;
    send_response(resp, idx);
    (void)frame;
}

/* FC03 - Read Holding Registers */
static void handle_fc03(uint8_t slave, const uint8_t *frame, uint16_t start, uint16_t qty)
{
    if (qty < 1 || qty > 125 || (start + qty) > REG_COUNT) {
        send_exception(slave, 0x03, 0x02);
        return;
    }
    uint8_t byte_count = qty * 2;
    uint8_t resp[256];
    size_t idx = 0;
    resp[idx++] = slave;
    resp[idx++] = 0x03;
    resp[idx++] = byte_count;
    for (uint16_t i = 0; i < qty; i++) {
        resp[idx++] = (holding_regs[start + i] >> 8) & 0xFF;
        resp[idx++] = holding_regs[start + i] & 0xFF;
    }
    uint16_t crc = crc16(resp, idx);
    resp[idx++] = crc & 0xFF;
    resp[idx++] = (crc >> 8) & 0xFF;
    send_response(resp, idx);
    (void)frame;
}

/* FC04 - Read Input Registers */
static void handle_fc04(uint8_t slave, const uint8_t *frame, uint16_t start, uint16_t qty)
{
    if (qty < 1 || qty > 125 || (start + qty) > REG_COUNT) {
        send_exception(slave, 0x04, 0x02);
        return;
    }
    uint8_t byte_count = qty * 2;
    uint8_t resp[256];
    size_t idx = 0;
    resp[idx++] = slave;
    resp[idx++] = 0x04;
    resp[idx++] = byte_count;
    for (uint16_t i = 0; i < qty; i++) {
        resp[idx++] = (input_regs[start + i] >> 8) & 0xFF;
        resp[idx++] = input_regs[start + i] & 0xFF;
    }
    uint16_t crc = crc16(resp, idx);
    resp[idx++] = crc & 0xFF;
    resp[idx++] = (crc >> 8) & 0xFF;
    send_response(resp, idx);
    (void)frame;
}

/* FC05 - Write Single Coil */
static void handle_fc05(uint8_t slave, const uint8_t *frame, uint16_t addr, uint16_t val)
{
    if (addr >= COIL_COUNT) {
        send_exception(slave, 0x05, 0x02);
        return;
    }
    coil_set(addr, val == 0xFF00);
    if (addr == 0) update_led();
    /* Echo back the request as response */
    uint8_t resp[8];
    resp[0] = slave;
    resp[1] = 0x05;
    resp[2] = (addr >> 8) & 0xFF;
    resp[3] = addr & 0xFF;
    resp[4] = coil_get(addr) ? 0xFF : 0x00;
    resp[5] = 0x00;
    uint16_t crc = crc16(resp, 6);
    resp[6] = crc & 0xFF;
    resp[7] = (crc >> 8) & 0xFF;
    send_response(resp, 8);
    ESP_LOGI(TAG, "Coil %u set to %s", addr, coil_get(addr) ? "ON" : "OFF");
    (void)frame;
}

/* FC06 - Write Single Register */
static void handle_fc06(uint8_t slave, const uint8_t *frame, uint16_t addr, uint16_t val)
{
    if (addr >= REG_COUNT) {
        send_exception(slave, 0x06, 0x02);
        return;
    }
    holding_regs[addr] = val;
    /* Echo back */
    uint8_t resp[8];
    resp[0] = slave;
    resp[1] = 0x06;
    resp[2] = (addr >> 8) & 0xFF;
    resp[3] = addr & 0xFF;
    resp[4] = (val >> 8) & 0xFF;
    resp[5] = val & 0xFF;
    uint16_t crc = crc16(resp, 6);
    resp[6] = crc & 0xFF;
    resp[7] = (crc >> 8) & 0xFF;
    send_response(resp, 8);
    ESP_LOGI(TAG, "Register %u set to %u", addr, val);
    (void)frame;
}

/* FC0F - Write Multiple Coils */
static void handle_fc0f(uint8_t slave, const uint8_t *frame, uint16_t start, uint16_t qty)
{
    if (qty < 1 || qty > 1968 || (start + qty) > COIL_COUNT) {
        send_exception(slave, 0x0F, 0x02);
        return;
    }
    uint8_t byte_count = frame[6];
    for (uint16_t i = 0; i < qty; i++) {
        bool val = (frame[7 + i / 8] >> (i % 8)) & 1;
        coil_set(start + i, val);
    }
    if (start == 0) update_led();
    uint8_t resp[8];
    resp[0] = slave;
    resp[1] = 0x0F;
    resp[2] = (start >> 8) & 0xFF;
    resp[3] = start & 0xFF;
    resp[4] = (qty >> 8) & 0xFF;
    resp[5] = qty & 0xFF;
    uint16_t crc = crc16(resp, 6);
    resp[6] = crc & 0xFF;
    resp[7] = (crc >> 8) & 0xFF;
    send_response(resp, 8);
    ESP_LOGI(TAG, "Write multiple coils: addr=%u, qty=%u, bytes=%u", start, qty, byte_count);
}

/* FC10 - Write Multiple Registers */
static void handle_fc10(uint8_t slave, const uint8_t *frame, uint16_t start, uint16_t qty)
{
    if (qty < 1 || qty > 123 || (start + qty) > REG_COUNT) {
        send_exception(slave, 0x10, 0x02);
        return;
    }
    uint8_t byte_count = frame[6];
    for (uint16_t i = 0; i < qty; i++) {
        uint16_t val = ((uint16_t)frame[7 + i * 2] << 8) |
                        frame[8 + i * 2];
        holding_regs[start + i] = val;
    }
    uint8_t resp[8];
    resp[0] = slave;
    resp[1] = 0x10;
    resp[2] = (start >> 8) & 0xFF;
    resp[3] = start & 0xFF;
    resp[4] = (qty >> 8) & 0xFF;
    resp[5] = qty & 0xFF;
    uint16_t crc = crc16(resp, 6);
    resp[6] = crc & 0xFF;
    resp[7] = (crc >> 8) & 0xFF;
    send_response(resp, 8);
    ESP_LOGI(TAG, "Write multiple regs: addr=%u, qty=%u, bytes=%u", start, qty, byte_count);
}

/* ---------- Frame processing ---------- */

static void process_frame(const uint8_t *frame, size_t len)
{
    if (len < 4) return; /* minimum: slave + fc + crc(2) */

    /* Verify CRC */
    uint16_t recv_crc = (uint16_t)frame[len - 2] | ((uint16_t)frame[len - 1] << 8);
    uint16_t calc_crc = crc16(frame, len - 2);
    if (recv_crc != calc_crc) {
        ESP_LOGW(TAG, "CRC mismatch: recv=0x%04X calc=0x%04X", recv_crc, calc_crc);
        return;
    }

    uint8_t slave = frame[0];
    if (slave != SLAVE_ID) return; /* not for us */

    uint8_t fc = frame[1];
    uint16_t start, qty, val;

    switch (fc) {
    case 0x01: /* Read Coils */
        if (len < 6) goto bad_len;
        start = ((uint16_t)frame[2] << 8) | frame[3];
        qty   = ((uint16_t)frame[4] << 8) | frame[5];
        handle_fc01(slave, frame, start, qty);
        break;

    case 0x02: /* Read Discrete Inputs */
        if (len < 6) goto bad_len;
        start = ((uint16_t)frame[2] << 8) | frame[3];
        qty   = ((uint16_t)frame[4] << 8) | frame[5];
        handle_fc02(slave, frame, start, qty);
        break;

    case 0x03: /* Read Holding Registers */
        if (len < 6) goto bad_len;
        start = ((uint16_t)frame[2] << 8) | frame[3];
        qty   = ((uint16_t)frame[4] << 8) | frame[5];
        handle_fc03(slave, frame, start, qty);
        break;

    case 0x04: /* Read Input Registers */
        if (len < 6) goto bad_len;
        start = ((uint16_t)frame[2] << 8) | frame[3];
        qty   = ((uint16_t)frame[4] << 8) | frame[5];
        handle_fc04(slave, frame, start, qty);
        break;

    case 0x05: /* Write Single Coil */
        if (len < 6) goto bad_len;
        start = ((uint16_t)frame[2] << 8) | frame[3];
        val   = ((uint16_t)frame[4] << 8) | frame[5];
        handle_fc05(slave, frame, start, val);
        break;

    case 0x06: /* Write Single Register */
        if (len < 6) goto bad_len;
        start = ((uint16_t)frame[2] << 8) | frame[3];
        val   = ((uint16_t)frame[4] << 8) | frame[5];
        handle_fc06(slave, frame, start, val);
        break;

    case 0x0F: /* Write Multiple Coils */
        if (len < 7) goto bad_len;
        start = ((uint16_t)frame[2] << 8) | frame[3];
        qty   = ((uint16_t)frame[4] << 8) | frame[5];
        handle_fc0f(slave, frame, start, qty);
        break;

    case 0x10: /* Write Multiple Registers */
        if (len < 7) goto bad_len;
        start = ((uint16_t)frame[2] << 8) | frame[3];
        qty   = ((uint16_t)frame[4] << 8) | frame[5];
        handle_fc10(slave, frame, start, qty);
        break;

    default:
        send_exception(slave, fc, 0x01); /* Illegal Function */
        break;
    }
    return;

bad_len:
    send_exception(slave, fc, 0x03); /* Illegal Data Value */
}

/* ---------- Data model update task ---------- */

static void data_update_task(void *arg)
{
    (void)arg;
    uint16_t counter = 0;

    while (1) {
        counter++;
        holding_regs[0] = counter;
        holding_regs[1] = (uint16_t)(esp_random() & 0xFFFF);

        /* Update some discrete inputs for variety */
        for (int i = 0; i < 8 && i < COIL_COUNT; i++) {
            if (coil_get(i)) {
                discrete_inputs[i / 8] |= (1 << (i % 8));
            } else {
                discrete_inputs[i / 8] &= ~(1 << (i % 8));
            }
        }

        vTaskDelay(pdMS_TO_TICKS(1000));
    }
}

/* ---------- UART event task ---------- */

static void uart_event_task(void *arg)
{
    (void)arg;

    uart_event_t event;
    uint8_t rx_buf[256];
    size_t rx_len = 0;

    while (1) {
        if (xQueueReceive(uart_queue, &event, pdMS_TO_TICKS(100))) {
            switch (event.type) {
            case UART_DATA:
                /* Read available data */
                {
                    int n = uart_read_bytes(UART_NUM, rx_buf + rx_len,
                                           sizeof(rx_buf) - rx_len,
                                           pdMS_TO_TICKS(UART_READ_TIMEOUT_MS));
                    if (n > 0) {
                        rx_len += n;
                    }
                }

                /* Check if we might have a complete frame.
                   Modbus RTU inter-character timeout is 3.5 characters.
                   At 9600 baud, 1 char ~ 1.04ms, so 3.5 chars ~ 3.6ms.
                   After the initial receive, wait a bit for silence. */
                {
                    size_t available = 0;
                    uart_get_buffered_data_len(UART_NUM, &available);
                    if (available == 0 && rx_len >= 4) {
                        /* No more data coming, process frame */
                        process_frame(rx_buf, rx_len);
                        rx_len = 0;
                    } else if (rx_len >= sizeof(rx_buf)) {
                        /* Buffer overflow, discard */
                        ESP_LOGW(TAG, "RX buffer overflow, discarding");
                        rx_len = 0;
                    }
                }
                break;

            case UART_FIFO_OVF:
                ESP_LOGW(TAG, "UART FIFO overflow");
                uart_flush_input(UART_NUM);
                rx_len = 0;
                break;

            case UART_BUFFER_FULL:
                ESP_LOGW(TAG, "UART buffer full");
                uart_flush_input(UART_NUM);
                rx_len = 0;
                break;

            case UART_BREAK:
            case UART_PARITY_ERR:
            case UART_FRAME_ERR:
                ESP_LOGW(TAG, "UART error: %d", event.type);
                uart_flush_input(UART_NUM);
                rx_len = 0;
                break;

            default:
                break;
            }
        } else {
            /* Timeout: if we have accumulated data with no new bytes,
               treat it as end-of-frame (inter-frame silence detected) */
            if (rx_len >= 4) {
                size_t available = 0;
                uart_get_buffered_data_len(UART_NUM, &available);
                if (available == 0) {
                    process_frame(rx_buf, rx_len);
                    rx_len = 0;
                }
            }
        }
    }
}

/* ---------- Public API ---------- */

esp_err_t modbus_rtu_slave_init(void)
{
    ESP_LOGI(TAG, "Modbus RTU Slave initializing (baud=%d, slave=%d, de=%d, tx=%d, rx=%d)",
             BAUD_RATE, SLAVE_ID, DE_PIN, TX_PIN, RX_PIN);

    /* Configure UART */
    uart_config_t uart_config = {
        .baud_rate = BAUD_RATE,
        .data_bits = UART_DATA_8_BITS,
        .parity = UART_PARITY_DISABLE,
        .stop_bits = UART_STOP_BITS_1,
        .flow_ctrl = UART_HW_FLOWCTRL_DISABLE,
        .source_clk = UART_SCLK_DEFAULT,
    };

    esp_err_t ret = uart_param_config(UART_NUM, &uart_config);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "uart_param_config failed: %s", esp_err_to_name(ret));
        return ret;
    }

    ret = uart_set_pin(UART_NUM, TX_PIN, RX_PIN, UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "uart_set_pin failed: %s", esp_err_to_name(ret));
        return ret;
    }

    ret = uart_driver_install(UART_NUM, UART_BUF_SIZE * 2, UART_BUF_SIZE * 2,
                              20, &uart_queue, 0);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "uart_driver_install failed: %s", esp_err_to_name(ret));
        return ret;
    }

    /* Configure DE pin for RS485 transceiver */
    gpio_config_t de_conf = {
        .pin_bit_mask = (1ULL << DE_PIN),
        .mode = GPIO_MODE_OUTPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    gpio_config(&de_conf);
    gpio_set_level(DE_PIN, 0); /* Start in RX mode */

    /* Configure LED pin */
    gpio_config_t led_conf = {
        .pin_bit_mask = (1ULL << LED_PIN),
        .mode = GPIO_MODE_OUTPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    gpio_config(&led_conf);
    gpio_set_level(LED_PIN, 0);

    /* Initialize data model */
    memset(holding_regs, 0, sizeof(holding_regs));
    memset(input_regs, 0, sizeof(input_regs));
    memset(coils, 0, sizeof(coils));
    memset(discrete_inputs, 0, sizeof(discrete_inputs));

    ESP_LOGI(TAG, "Modbus RTU Slave initialized");
    return ESP_OK;
}

esp_err_t modbus_rtu_slave_start(void)
{
    xTaskCreate(uart_event_task, "mb_rtu_slave",
                4096, NULL, 5, NULL);
    xTaskCreate(data_update_task, "mb_data_update",
                2048, NULL, 4, NULL);
    ESP_LOGI(TAG, "Modbus RTU Slave tasks started");
    return ESP_OK;
}
