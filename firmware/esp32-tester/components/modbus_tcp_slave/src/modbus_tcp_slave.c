#include "modbus_tcp_slave.h"
#include "board.h"
#include "esp_log.h"
#include "esp_random.h"
#include "esp_netif.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "mbcontroller.h"
#include <string.h>

static const char *TAG = "modbus_tcp_slave";

#define TCP_PORT     CONFIG_MODBUS_SLAVE_PORT
#define SLAVE_ID     CONFIG_MODBUS_SLAVE_ID
#define NUM_REGISTERS CONFIG_MODBUS_SLAVE_NUM_REGISTERS
#define NUM_COILS    6

// Register map:
//   0     = auto-incrementing counter
//   1     = random value (0-999)
//   10    = buzzer (0=off, 1=on)
//   11    = RGB Red (0-255)
//   12    = RGB Green (0-255)
//   13    = RGB Blue (0-255)
//   14-99 = general purpose

static uint16_t holding_regs[NUM_REGISTERS];
static uint16_t input_regs[NUM_REGISTERS];
static uint8_t coils[4];
static uint8_t discrete_inputs[4];
static bool running = false;

#define MB_WRITE_MASK (MB_EVENT_HOLDING_REG_WR | MB_EVENT_COILS_WR)
#define MB_RW_MASK    (MB_WRITE_MASK | MB_EVENT_HOLDING_REG_RD | MB_EVENT_INPUT_REG_RD \
                       | MB_EVENT_DISCRETE_RD | MB_EVENT_COILS_RD)

static void apply_register_write(uint16_t addr, uint16_t value) {
    holding_regs[addr] = value;
    switch (addr) {
    case 10: board_set_buzzer(value != 0); break;
    case 11: board_set_rgb(value & 0xFF, holding_regs[12] & 0xFF, holding_regs[13] & 0xFF); break;
    case 12: board_set_rgb(holding_regs[11] & 0xFF, value & 0xFF, holding_regs[13] & 0xFF); break;
    case 13: board_set_rgb(holding_regs[11] & 0xFF, holding_regs[12] & 0xFF, value & 0xFF); break;
    }
}

static void apply_coil_change(void) {
    for (int i = 0; i < NUM_COILS; i++) {
        bool val = (coils[i / 8] >> (i % 8)) & 1;
        board_set_relay(i, val);
    }
}

esp_err_t modbus_tcp_slave_init(esp_netif_t *netif) {
    memset(holding_regs, 0, sizeof(holding_regs));
    memset(coils, 0, sizeof(coils));
    memset(discrete_inputs, 0, sizeof(discrete_inputs));

    board_init();
    holding_regs[1] = 123;

    void *slave_handler = NULL;
    esp_err_t err = mbc_slave_init_tcp(&slave_handler);
    if (err != ESP_OK || slave_handler == NULL) {
        ESP_LOGE(TAG, "mbc_slave_init_tcp failed: 0x%x", err);
        return err;
    }

    mb_communication_info_t comm_info = {0};
    comm_info.ip_addr = NULL;
    comm_info.ip_netif_ptr = (void *)netif;
    comm_info.slave_uid = SLAVE_ID;
    comm_info.ip_addr_type = MB_IPV4;
    comm_info.ip_mode = MB_MODE_TCP;
    comm_info.ip_port = TCP_PORT;

    err = mbc_slave_setup((void *)&comm_info);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "mbc_slave_setup failed: 0x%x", err);
        return err;
    }

    mb_register_area_descriptor_t reg_area;

    reg_area.type = MB_PARAM_HOLDING;
    reg_area.start_offset = 0;
    reg_area.address = (void *)holding_regs;
    reg_area.size = sizeof(holding_regs);
    ESP_ERROR_CHECK(mbc_slave_set_descriptor(reg_area));

    reg_area.type = MB_PARAM_INPUT;
    reg_area.start_offset = 0;
    reg_area.address = (void *)input_regs;
    reg_area.size = sizeof(input_regs);
    ESP_ERROR_CHECK(mbc_slave_set_descriptor(reg_area));

    reg_area.type = MB_PARAM_COIL;
    reg_area.start_offset = 0;
    reg_area.address = (void *)coils;
    reg_area.size = sizeof(coils);
    ESP_ERROR_CHECK(mbc_slave_set_descriptor(reg_area));

    reg_area.type = MB_PARAM_DISCRETE;
    reg_area.start_offset = 0;
    reg_area.address = (void *)discrete_inputs;
    reg_area.size = sizeof(discrete_inputs);
    ESP_ERROR_CHECK(mbc_slave_set_descriptor(reg_area));

    running = true;
    ESP_LOGI(TAG, "Modbus TCP Slave init (id=%d, port=%d, regs=%d, coils=%d)",
             SLAVE_ID, TCP_PORT, NUM_REGISTERS, NUM_COILS);
    return ESP_OK;
}

static void event_task(void *arg) {
    ESP_ERROR_CHECK(mbc_slave_start());

    while (running) {
        (void)mbc_slave_check_event(MB_RW_MASK);

        mb_param_info_t reg_info;
        esp_err_t err = mbc_slave_get_param_info(&reg_info, 10);
        if (err != ESP_OK) continue;

        if (reg_info.type & MB_EVENT_COILS_WR) {
            ESP_LOGI(TAG, "COIL WR offset=%u size=%u addr=%p data=0x%02X",
                     reg_info.mb_offset, reg_info.size, reg_info.address, coils[0]);
            apply_coil_change();
        }
        if (reg_info.type & MB_EVENT_HOLDING_REG_WR) {
            uint16_t offset = reg_info.mb_offset;
            uint16_t count = reg_info.size; // size is already in registers
            for (int i = 0; i < count && (offset + i) < NUM_REGISTERS; i++) {
                apply_register_write(offset + i, holding_regs[offset + i]);
            }
        }
    }
    vTaskDelete(NULL);
}

static void data_update_task(void *arg) {
    uint16_t counter = 0;
    while (running) {
        counter++;
        holding_regs[0] = counter;
        holding_regs[1] = (uint16_t)(esp_random() % 1000);
        memcpy(discrete_inputs, coils, sizeof(coils));
        memcpy(input_regs, holding_regs, sizeof(holding_regs));
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    vTaskDelete(NULL);
}

esp_err_t modbus_tcp_slave_start(void) {
    xTaskCreate(event_task, "mb_evt", 4096, NULL, tskIDLE_PRIORITY + 5, NULL);
    xTaskCreate(data_update_task, "mb_data", 4096, NULL, tskIDLE_PRIORITY + 3, NULL);
    return ESP_OK;
}
