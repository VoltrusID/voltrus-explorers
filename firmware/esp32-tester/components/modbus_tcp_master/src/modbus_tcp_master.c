#include "modbus_tcp_master.h"

#include <errno.h>
#include <string.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>

#include "esp_log.h"
#include "esp_system.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "modbus_tcp_master";

/* MBAP header + PDU limits */
#define MBAP_HEADER_LEN  7
#define MAX_PDU_LEN      253
#define MAX_ADU_LEN      (MBAP_HEADER_LEN + MAX_PDU_LEN)
#define MODBUS_TCP_PORT  CONFIG_MODBUS_MASTER_SERVER_PORT
#define POLL_INTERVAL_MS CONFIG_MODBUS_MASTER_POLL_INTERVAL_MS
#define TARGET_SLAVE_ID  1

static int sock_fd = -1;
static uint16_t transaction_id = 0;
static bool coil_state = false;

/* ---------- Modbus TCP helpers ---------- */

static uint16_t next_tid(void)
{
    return ++transaction_id;
}

/* Build a Modbus TCP ADU (MBAP header + PDU).
   Returns total ADU length, or -1 on error. */
static int build_adu(uint8_t *adu, size_t adu_cap,
                     uint8_t slave_id, uint8_t fc,
                     uint16_t start_addr, uint16_t quantity_or_value)
{
    uint16_t pdu_len;

    switch (fc) {
    case 0x03: /* Read Holding Registers */
    case 0x04: /* Read Input Registers */
    case 0x01: /* Read Coils */
    case 0x02: /* Read Discrete Inputs */
        pdu_len = 5; /* slave(1) + fc(1) + start(2) + qty(2) */
        break;
    case 0x05: /* Write Single Coil */
    case 0x06: /* Write Single Register */
        pdu_len = 5; /* slave(1) + fc(1) + addr(2) + value(2) */
        break;
    case 0x0F: /* Write Multiple Coils */
    case 0x10: /* Write Multiple Registers */
        /* Not used in this component, but stubbed for completeness */
        pdu_len = 6; /* minimum: slave+fc+start+qty+bytecount+1byte */
        break;
    default:
        return -1;
    }

    uint16_t total = MBAP_HEADER_LEN + pdu_len;
    if (total > adu_cap) return -1;

    uint16_t tid = next_tid();
    adu[0] = (tid >> 8) & 0xFF;
    adu[1] = tid & 0xFF;
    adu[2] = 0x00; /* protocol ID high */
    adu[3] = 0x00; /* protocol ID low */
    adu[4] = ((pdu_len + 1) >> 8) & 0xFF; /* length high (unit + PDU) */
    adu[5] = (pdu_len + 1) & 0xFF;        /* length low */
    adu[6] = slave_id;
    adu[7] = fc;
    adu[8] = (start_addr >> 8) & 0xFF;
    adu[9] = start_addr & 0xFF;

    if (fc == 0x05) {
        /* Write Single Coil: 0xFF00 = ON, 0x0000 = OFF */
        uint16_t coil_val = quantity_or_value ? 0xFF00 : 0x0000;
        adu[10] = (coil_val >> 8) & 0xFF;
        adu[11] = coil_val & 0xFF;
    } else {
        adu[10] = (quantity_or_value >> 8) & 0xFF;
        adu[11] = quantity_or_value & 0xFF;
    }

    return (int)total;
}

/* Send ADU and receive response. Returns response ADU length, or -1 on error. */
static int send_recv(const uint8_t *req, int req_len,
                     uint8_t *resp, size_t resp_cap)
{
    if (sock_fd < 0) return -1;

    /* Send request */
    int sent = send(sock_fd, req, req_len, 0);
    if (sent != req_len) {
        ESP_LOGE(TAG, "send() failed: errno %d (%s)", errno, strerror(errno));
        return -1;
    }

    /* Read MBAP header first (7 bytes) */
    int total_read = 0;
    while (total_read < MBAP_HEADER_LEN) {
        int n = recv(sock_fd, resp + total_read,
                     MBAP_HEADER_LEN - total_read, 0);
        if (n <= 0) {
            ESP_LOGE(TAG, "recv(MBAP) failed: errno %d (%s)", errno,
                     strerror(errno));
            return -1;
        }
        total_read += n;
    }

    /* Extract PDU length from MBAP header */
    uint16_t resp_pdu_len = ((uint16_t)resp[4] << 8) | resp[5];
    if (resp_pdu_len == 0 || (MBAP_HEADER_LEN + resp_pdu_len) > resp_cap) {
        ESP_LOGE(TAG, "Invalid response length: %u", resp_pdu_len);
        return -1;
    }

    /* Read remaining PDU */
    int remaining = resp_pdu_len;
    while (remaining > 0) {
        int n = recv(sock_fd, resp + total_read, remaining, 0);
        if (n <= 0) {
            ESP_LOGE(TAG, "recv(PDU) failed: errno %d (%s)", errno,
                     strerror(errno));
            return -1;
        }
        total_read += n;
        remaining -= n;
    }

    return total_read;
}

/* ---------- Connection management ---------- */

static int connect_to_server(void)
{
    struct sockaddr_in dest_addr = {
        .sin_family = AF_INET,
        .sin_port = htons(MODBUS_TCP_PORT),
    };
    inet_aton(CONFIG_MODBUS_MASTER_SERVER_IP, &dest_addr.sin_addr);

    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
        ESP_LOGE(TAG, "socket() failed: errno %d (%s)", errno, strerror(errno));
        return -1;
    }

    ESP_LOGI(TAG, "Connecting to %s:%d ...",
             inet_ntoa(dest_addr.sin_addr), MODBUS_TCP_PORT);

    int ret = connect(fd, (struct sockaddr *)&dest_addr, sizeof(dest_addr));
    if (ret != 0) {
        ESP_LOGE(TAG, "connect() failed: errno %d (%s)", errno, strerror(errno));
        close(fd);
        return -1;
    }

    ESP_LOGI(TAG, "Connected to Modbus TCP server");
    return fd;
}

static void disconnect_server(void)
{
    if (sock_fd >= 0) {
        close(sock_fd);
        sock_fd = -1;
        ESP_LOGI(TAG, "Disconnected from server");
    }
}

/* ---------- Poll functions ---------- */

static esp_err_t poll_holding_registers(void)
{
    uint8_t req[MAX_ADU_LEN];
    uint8_t resp[MAX_ADU_LEN];

    int req_len = build_adu(req, sizeof(req),
                            TARGET_SLAVE_ID, 0x03, 0, 10);
    if (req_len < 0) return ESP_FAIL;

    int resp_len = send_recv(req, req_len, resp, sizeof(resp));
    if (resp_len < 0) return ESP_FAIL;

    /* Validate: FC=0x03, check exception */
    uint8_t fc = resp[7];
    if (fc & 0x80) {
        ESP_LOGE(TAG, "Modbus exception: FC=0x%02X, code=0x%02X",
                 fc, resp[8]);
        return ESP_FAIL;
    }

    uint8_t byte_count = resp[8];
    if (byte_count != 20) { /* 10 registers * 2 bytes */
        ESP_LOGW(TAG, "Unexpected byte count: %u", byte_count);
    }

    ESP_LOGI(TAG, "Holding registers (addr 0, count %d):",
             byte_count / 2);
    for (int i = 0; i < byte_count / 2 && i < 10; i++) {
        uint16_t val = ((uint16_t)resp[9 + i * 2] << 8) |
                        resp[10 + i * 2];
        ESP_LOGI(TAG, "  Reg[%d] = %u (0x%04X)", i, val, val);
    }

    return ESP_OK;
}

static esp_err_t toggle_coil(void)
{
    uint8_t req[MAX_ADU_LEN];
    uint8_t resp[MAX_ADU_LEN];

    bool new_state = !coil_state;

    int req_len = build_adu(req, sizeof(req),
                            TARGET_SLAVE_ID, 0x05, 0,
                            new_state ? 1 : 0);
    if (req_len < 0) return ESP_FAIL;

    int resp_len = send_recv(req, req_len, resp, sizeof(resp));
    if (resp_len < 0) return ESP_FAIL;

    uint8_t fc = resp[7];
    if (fc & 0x80) {
        ESP_LOGE(TAG, "Write coil exception: FC=0x%02X, code=0x%02X",
                 fc, resp[8]);
        return ESP_FAIL;
    }

    /* Toggle state only after write confirmed successful */
    coil_state = new_state;
    ESP_LOGI(TAG, "Coil 0 set to %s", coil_state ? "ON" : "OFF");

    /* Suppress unused warning for resp_len (we checked for error above) */
    (void)resp_len;
    return ESP_OK;
}

/* ---------- Main task ---------- */

static void modbus_master_task(void *arg)
{
    (void)arg;

    while (1) {
        /* Connect if needed */
        if (sock_fd < 0) {
            sock_fd = connect_to_server();
            if (sock_fd < 0) {
                ESP_LOGW(TAG, "Connection failed, retrying in %d ms...",
                         POLL_INTERVAL_MS);
                vTaskDelay(pdMS_TO_TICKS(POLL_INTERVAL_MS));
                continue;
            }
        }

        /* Poll holding registers */
        esp_err_t ret = poll_holding_registers();
        if (ret != ESP_OK) {
            ESP_LOGW(TAG, "Register poll failed, reconnecting...");
            disconnect_server();
            vTaskDelay(pdMS_TO_TICKS(POLL_INTERVAL_MS));
            continue;
        }

        /* Toggle coil 0 */
        ret = toggle_coil();
        if (ret != ESP_OK) {
            ESP_LOGW(TAG, "Coil write failed, reconnecting...");
            disconnect_server();
            vTaskDelay(pdMS_TO_TICKS(POLL_INTERVAL_MS));
            continue;
        }

        vTaskDelay(pdMS_TO_TICKS(POLL_INTERVAL_MS));
    }
}

/* ---------- Public API ---------- */

esp_err_t modbus_tcp_master_init(void)
{
    ESP_LOGI(TAG, "Modbus TCP Master initialized (target: %s:%d, interval: %d ms)",
             CONFIG_MODBUS_MASTER_SERVER_IP,
             CONFIG_MODBUS_MASTER_SERVER_PORT,
             CONFIG_MODBUS_MASTER_POLL_INTERVAL_MS);
    return ESP_OK;
}

esp_err_t modbus_tcp_master_start(void)
{
    xTaskCreate(modbus_master_task, "mb_tcp_master",
                4096, NULL, 5, NULL);
    ESP_LOGI(TAG, "Modbus TCP Master task started");
    return ESP_OK;
}
