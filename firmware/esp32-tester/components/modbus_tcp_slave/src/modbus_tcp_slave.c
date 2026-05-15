#include "modbus_tcp_slave.h"
#include "board.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"
#include "lwip/netdb.h"
#include "lwip/inet.h"
#include <string.h>
#include <stdlib.h>
#include <errno.h>

static const char *TAG = "modbus_tcp_slave";

#define TCP_PORT CONFIG_MODBUS_SLAVE_PORT
#define SLAVE_ID CONFIG_MODBUS_SLAVE_ID
#define NUM_REGISTERS CONFIG_MODBUS_SLAVE_NUM_REGISTERS
#define NUM_COILS 6
#define MAX_CLIENTS 5
#define RX_BUFFER_SIZE 256
#define TX_BUFFER_SIZE 512

// Register map:
//   0     = auto-incrementing counter
//   1     = random value (0-999)
//   10    = buzzer (0=off, 1=on)
//   11    = RGB Red (0-255)
//   12    = RGB Green (0-255)
//   13    = RGB Blue (0-255)
//   14-99 = general purpose

static uint16_t holding_regs[NUM_REGISTERS];
static uint8_t coils[(NUM_COILS + 7) / 8];
static uint16_t counter = 0;
static bool running = false;

#define FC_READ_COILS              0x01
#define FC_READ_DISCRETE_INPUTS    0x02
#define FC_READ_HOLDING_REGS       0x03
#define FC_READ_INPUT_REGS         0x04
#define FC_WRITE_SINGLE_COIL       0x05
#define FC_WRITE_SINGLE_REG        0x06
#define FC_WRITE_MULTIPLE_REGS     0x10
#define FC_WRITE_MULTIPLE_COILS    0x0F

#define EX_ILLEGAL_FUNCTION        0x01
#define EX_ILLEGAL_DATA_ADDR       0x02
#define EX_ILLEGAL_DATA_VALUE      0x03
#define EX_SERVER_DEVICE_FAILURE   0x04

static void set_coil(int addr, bool value) {
    if (addr >= NUM_COILS) return;
    if (value) {
        coils[addr / 8] |= (1 << (addr % 8));
    } else {
        coils[addr / 8] &= ~(1 << (addr % 8));
    }
}

static bool get_coil(int addr) {
    if (addr >= NUM_COILS) return false;
    return (coils[addr / 8] >> (addr % 8)) & 1;
}

static int build_exception(uint8_t *req, uint8_t *resp, uint8_t ex_code) {
    resp[0] = req[0];
    resp[1] = req[1];
    resp[2] = 0;
    resp[3] = 0;
    resp[4] = 0;
    resp[5] = 3;
    resp[6] = req[6];
    resp[7] = req[7] | 0x80;
    resp[8] = ex_code;
    return 9;
}

static void apply_register_write(uint16_t addr, uint16_t value) {
    holding_regs[addr] = value;
    switch (addr) {
    case 10: board_set_buzzer(value != 0); break;
    case 11: board_set_rgb(value & 0xFF, holding_regs[12] & 0xFF, holding_regs[13] & 0xFF); break;
    case 12: board_set_rgb(holding_regs[11] & 0xFF, value & 0xFF, holding_regs[13] & 0xFF); break;
    case 13: board_set_rgb(holding_regs[11] & 0xFF, holding_regs[12] & 0xFF, value & 0xFF); break;
    }
}

static int process_request(uint8_t *req, int req_len, uint8_t *resp) {
    if (req_len < 8) return 0;

    uint16_t protocol = (req[2] << 8) | req[3];
    uint8_t unit_id = req[6];
    uint8_t func = req[7];

    if (protocol != 0) return 0;
    if (unit_id != SLAVE_ID) return 0;

    resp[0] = req[0];
    resp[1] = req[1];
    resp[2] = 0;
    resp[3] = 0;
    resp[6] = unit_id;
    resp[7] = func;

    switch (func) {
    case FC_READ_COILS:
    case FC_READ_DISCRETE_INPUTS: {
        uint16_t start = (req[8] << 8) | req[9];
        uint16_t count = (req[10] << 8) | req[11];
        if (count < 1 || count > 2000 || start + count > NUM_COILS) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_ADDR);
        }
        uint8_t byte_count = (count + 7) / 8;
        resp[2] = 0; resp[3] = 0;
        resp[4] = 0; resp[5] = 3 + byte_count;
        resp[8] = byte_count;
        memset(&resp[9], 0, byte_count);
        for (int i = 0; i < count; i++) {
            if (get_coil(start + i)) {
                resp[9 + i / 8] |= (1 << (i % 8));
            }
        }
        return 9 + byte_count;
    }
    case FC_READ_HOLDING_REGS:
    case FC_READ_INPUT_REGS: {
        uint16_t start = (req[8] << 8) | req[9];
        uint16_t count = (req[10] << 8) | req[11];
        if (count < 1 || count > 125 || start + count > NUM_REGISTERS) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_ADDR);
        }
        uint8_t byte_count = count * 2;
        resp[2] = 0; resp[3] = 0;
        resp[4] = 0; resp[5] = 3 + byte_count;
        resp[8] = byte_count;
        for (int i = 0; i < count; i++) {
            resp[9 + i * 2] = (holding_regs[start + i] >> 8) & 0xFF;
            resp[10 + i * 2] = holding_regs[start + i] & 0xFF;
        }
        return 9 + byte_count;
    }
    case FC_WRITE_SINGLE_COIL: {
        uint16_t addr = (req[8] << 8) | req[9];
        uint16_t value = (req[10] << 8) | req[11];
        if (addr >= NUM_COILS) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_ADDR);
        }
        set_coil(addr, value == 0xFF00);
        board_set_relay(addr, value == 0xFF00);
        memcpy(&resp[2], &req[2], 6);
        resp[4] = 0; resp[5] = 6;
        return 12;
    }
    case FC_WRITE_SINGLE_REG: {
        uint16_t addr = (req[8] << 8) | req[9];
        uint16_t value = (req[10] << 8) | req[11];
        if (addr >= NUM_REGISTERS) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_ADDR);
        }
        apply_register_write(addr, value);
        memcpy(&resp[2], &req[2], 6);
        resp[4] = 0; resp[5] = 6;
        return 12;
    }
    case FC_WRITE_MULTIPLE_REGS: {
        uint16_t start = (req[8] << 8) | req[9];
        uint16_t count = (req[10] << 8) | req[11];
        if (start + count > NUM_REGISTERS) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_ADDR);
        }
        uint8_t byte_count = req[12];
        if (byte_count != count * 2) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_VALUE);
        }
        if (req_len < 13 + byte_count) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_VALUE);
        }
        for (int i = 0; i < count; i++) {
            apply_register_write(start + i, (req[13 + i * 2] << 8) | req[14 + i * 2]);
        }
        memcpy(&resp[2], &req[2], 6);
        resp[4] = 0; resp[5] = 6;
        return 12;
    }
    case FC_WRITE_MULTIPLE_COILS: {
        uint16_t start = (req[8] << 8) | req[9];
        uint16_t count = (req[10] << 8) | req[11];
        if (start + count > NUM_COILS) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_ADDR);
        }
        uint8_t byte_count = req[12];
        uint8_t expected = (count + 7) / 8;
        if (byte_count != expected) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_VALUE);
        }
        if (req_len < 13 + byte_count) {
            return build_exception(req, resp, EX_ILLEGAL_DATA_VALUE);
        }
        for (int i = 0; i < count; i++) {
            bool val = (req[13 + i / 8] >> (i % 8)) & 1;
            set_coil(start + i, val);
            board_set_relay(start + i, val);
        }
        memcpy(&resp[2], &req[2], 6);
        resp[4] = 0; resp[5] = 6;
        return 12;
    }
    default:
        return build_exception(req, resp, EX_ILLEGAL_FUNCTION);
    }
}

static void handle_client(int client_fd) {
    uint8_t rx_buf[RX_BUFFER_SIZE];
    uint8_t tx_buf[TX_BUFFER_SIZE];

    while (running) {
        int received = recv(client_fd, rx_buf, RX_BUFFER_SIZE, 0);
        if (received <= 0) {
            ESP_LOGW(TAG, "client disconnected (fd=%d)", client_fd);
            break;
        }

        if (received < 8) continue;

        uint16_t pdu_len = (rx_buf[4] << 8) | rx_buf[5];
        int total_expected = 6 + pdu_len;

        int total_received = received;
        while (total_received < total_expected && total_received < RX_BUFFER_SIZE) {
            int r = recv(client_fd, &rx_buf[total_received],
                         total_expected - total_received, 0);
            if (r <= 0) break;
            total_received += r;
        }

        int resp_len = process_request(rx_buf, total_received, tx_buf);
        if (resp_len > 0) {
            send(client_fd, tx_buf, resp_len, 0);
        }
    }
    close(client_fd);
}

static void data_update_task(void *arg) {
    while (running) {
        counter++;
        holding_regs[0] = counter;
        holding_regs[1] = (uint16_t)(esp_random() % 1000);
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    vTaskDelete(NULL);
}

static void tcp_server_task(void *arg) {
    struct sockaddr_in dest_addr;
    dest_addr.sin_addr.s_addr = htonl(INADDR_ANY);
    dest_addr.sin_family = AF_INET;
    dest_addr.sin_port = htons(TCP_PORT);

    int listen_fd = socket(AF_INET, SOCK_STREAM, IPPROTO_IP);
    if (listen_fd < 0) {
        ESP_LOGE(TAG, "unable to create socket: errno %d", errno);
        vTaskDelete(NULL);
        return;
    }

    int opt = 1;
    setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));

    if (bind(listen_fd, (struct sockaddr *)&dest_addr, sizeof(dest_addr)) != 0) {
        ESP_LOGE(TAG, "socket bind failed: errno %d", errno);
        close(listen_fd);
        vTaskDelete(NULL);
        return;
    }

    if (listen(listen_fd, MAX_CLIENTS) != 0) {
        ESP_LOGE(TAG, "error during listen: errno %d", errno);
        close(listen_fd);
        vTaskDelete(NULL);
        return;
    }

    ESP_LOGI(TAG, "Modbus TCP slave listening on port %d", TCP_PORT);

    while (running) {
        struct sockaddr_in source_addr;
        socklen_t addr_len = sizeof(source_addr);
        int client_fd = accept(listen_fd, (struct sockaddr *)&source_addr, &addr_len);
        if (client_fd < 0) {
            if (running) ESP_LOGE(TAG, "accept failed: errno %d", errno);
            break;
        }
        ESP_LOGI(TAG, "client connected from %s:%d",
                 inet_ntoa(source_addr.sin_addr), ntohs(source_addr.sin_port));
        handle_client(client_fd);
    }

    close(listen_fd);
    vTaskDelete(NULL);
}

esp_err_t modbus_tcp_slave_init(void) {
    memset(holding_regs, 0, sizeof(holding_regs));
    memset(coils, 0, sizeof(coils));

    board_init();

    holding_regs[1] = 123;
    running = true;

    ESP_LOGI(TAG, "Modbus TCP Slave initialized (slave_id=%d, port=%d, regs=%d, coils=%d)",
             SLAVE_ID, TCP_PORT, NUM_REGISTERS, NUM_COILS);
    return ESP_OK;
}

esp_err_t modbus_tcp_slave_start(void) {
    xTaskCreate(tcp_server_task, "mb_tcp_slave", 4096, NULL,
                tskIDLE_PRIORITY + 5, NULL);
    xTaskCreate(data_update_task, "mb_data_upd", 2048, NULL,
                tskIDLE_PRIORITY + 3, NULL);
    return ESP_OK;
}
