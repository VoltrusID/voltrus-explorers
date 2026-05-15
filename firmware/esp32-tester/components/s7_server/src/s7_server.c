#include "s7_server.h"
#include "esp_log.h"
#include "esp_random.h"
#include "board.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"
#include "lwip/inet.h"
#include <string.h>
#include <stdlib.h>
#include <errno.h>

static const char *TAG = "s7_server";

#define TCP_PORT CONFIG_S7_SERVER_PORT
#define S7_RACK  CONFIG_S7_RACK
#define S7_SLOT  CONFIG_S7_SLOT
#define DB1_SIZE 64
#define NUM_DB1_WORDS (DB1_SIZE / 2)
#define RX_BUFFER_SIZE 2048
#define MAX_CLIENTS 5

static uint8_t db1[DB1_SIZE];
static uint16_t counter = 0;
static bool running = false;

/* TPKT header: version(1) reserved(1) length(2) */
#define TPKT_HEADER_SIZE 4

/* COTP types */
#define COTP_CR 0xE0  /* Connection Request */
#define COTP_CC 0xD0  /* Connection Confirm */
#define COTP_DT 0xF0  /* Data */

/* S7 message types */
#define S7_MSG_TYPE_JOB        0x01
#define S7_MSG_TYPE_ACK_DATA   0x03

/* S7 function codes */
#define S7_FC_SETUP_COMM       0xF0
#define S7_FC_READ_VAR         0x04
#define S7_FC_WRITE_VAR        0x05

/* S7 error codes */
#define S7_ERR_NO_ERROR        0x00
#define S7_ERR_NOT_FOUND       0x03
#define S7_ERR_FAULT           0x06

/* S7 transport sizes */
#define S7_TS_BIT   0x03
#define S7_TS_BYTE  0x04
#define S7_TS_WORD  0x05
#define S7_TS_INT   0x05
#define S7_TS_REAL  0x07
#define S7_TS_COUNTER 0x1C

/* S7 area codes */
#define S7_AREA_PE  0x81
#define S7_AREA_PA  0x82
#define S7_AREA_MK  0x83
#define S7_AREA_DB  0x84
#define S7_AREA_CT  0x1C
#define S7_AREA_TM  0x1D

static uint16_t get_u16_be(const uint8_t *p)
{
    return ((uint16_t)p[0] << 8) | p[1];
}

static void put_u16_be(uint8_t *p, uint16_t v)
{
    p[0] = (v >> 8) & 0xFF;
    p[1] = v & 0xFF;
}

/* Build TPKT + COTP DT header. Returns total header size. */
static int build_s7_header(uint8_t *buf, uint16_t s7_data_len)
{
    int cotp_len = 3; /* COTP DT: header_len(1) + type(1) + seq(1) */
    int tpkt_payload = cotp_len + s7_data_len;
    int total = TPKT_HEADER_SIZE + tpkt_payload;

    /* TPKT */
    buf[0] = 0x03; /* version */
    buf[1] = 0x00; /* reserved */
    put_u16_be(&buf[2], total);

    /* COTP DT */
    buf[4] = 0x02; /* header length */
    buf[5] = COTP_DT;
    buf[6] = 0x80; /* seq: last + EOT */

    return TPKT_HEADER_SIZE + cotp_len;
}

/* Handle COTP Connection Request, build COTP Connection Confirm */
static int handle_cotp_cr(const uint8_t *req, int req_len, uint8_t *resp)
{
    if (req_len < TPKT_HEADER_SIZE + 6) {
        return 0;
    }

    /* Build TPKT + COTP CC */
    int cc_len = 6; /* src-ref(2) + dst-ref(2) + class(1) + opt(1) */
    int tpkt_total = TPKT_HEADER_SIZE + cc_len;

    /* TPKT */
    resp[0] = 0x03;
    resp[1] = 0x00;
    put_u16_be(&resp[2], tpkt_total);

    /* COTP CC */
    resp[4] = cc_len - 1; /* header length excluding this byte */
    resp[5] = COTP_CC;
    put_u16_be(&resp[6], 0x0001); /* dst-ref */
    put_u16_be(&resp[8], 0x0001); /* src-ref */
    resp[10] = 0x00; /* class */

    return tpkt_total;
}

/* Build S7 Setup Communication response */
static int build_setup_comm_response(const uint8_t *req, uint8_t *resp_s7)
{
    (void)req;
    /* S7 header: 12 bytes */
    resp_s7[0] = 0x32;        /* protocol ID */
    resp_s7[1] = S7_MSG_TYPE_ACK_DATA;
    put_u16_be(&resp_s7[2], 0x0000); /* reserved */
    put_u16_be(&resp_s7[4], get_u16_be(req + 4)); /* req PDU ref */
    put_u16_be(&resp_s7[6], 0x0000); /* parameter length */
    put_u16_be(&resp_s7[8], 0x0008); /* data length */
    resp_s7[10] = 0x00;       /* error class */
    resp_s7[11] = 0x00;       /* error code */

    /* Data: setup comm parameters (8 bytes) */
    put_u16_be(&resp_s7[12], 0x0000); /* reserved */
    put_u16_be(&resp_s7[14], 0x0100); /* max AMQ caller */
    put_u16_be(&resp_s7[16], 0x0100); /* max AMQ callee */
    put_u16_be(&resp_s7[18], 0x03C0); /* PDU size (960) */

    return 20;
}

/* Parse S7 item address specification (S7 Any pointer) */
static bool parse_s7_any(const uint8_t *p, uint8_t *area, uint8_t *transport_size,
                         uint16_t *db_number, uint32_t *byte_offset, uint16_t *item_length)
{
    *transport_size = p[1];
    *item_length = get_u16_be(&p[2]);
    *db_number = get_u16_be(&p[4]);
    *area = p[6];
    /* byte offset: bits 23-3 of the 3-byte address field */
    *byte_offset = ((uint32_t)p[7] << 16) | ((uint32_t)p[8] << 8) | p[9];
    *byte_offset >>= 3; /* convert bit address to byte address */
    return true;
}

/* Build S7 Read Var response */
static int build_read_var_response(const uint8_t *req, int req_len, uint8_t *resp_s7)
{
    if (req_len < 14) return 0;

    uint8_t num_items = req[13]; /* parameter: item count */
    int param_len = 2; /* function code + item count */
    int data_offset = 14; /* after S7 header(12) + param(2) */

    /* Calculate data length needed for response */
    int resp_data_len = 1; /* error code byte */
    for (int i = 0; i < num_items && data_offset + 9 <= req_len; i++) {
        uint8_t ts;
        uint16_t item_len, db_num;
        uint8_t area;
        uint32_t byte_off;

        parse_s7_any(&req[data_offset], &area, &ts, &db_num, &byte_off, &item_len);
        data_offset += 12; /* S7 Any is 12 bytes (but we index from item start) */

        /* Fix: S7 Any is actually read from offset +1 because the first byte
           after param_len is the item spec type (0x12 for S7 Any).
           Let's re-parse properly. */
        (void)ts;
        (void)db_num;

        int byte_count = item_len;

        /* For word type, item_len is in elements (words), each word = 2 bytes */
        resp_data_len += 1 + byte_count; /* return code + data */
    }

    /* S7 header (12 bytes) */
    resp_s7[0] = 0x32;
    resp_s7[1] = S7_MSG_TYPE_ACK_DATA;
    put_u16_be(&resp_s7[2], 0x0000);
    put_u16_be(&resp_s7[4], get_u16_be(req + 4)); /* PDU ref */
    put_u16_be(&resp_s7[6], param_len);
    put_u16_be(&resp_s7[8], resp_data_len);
    resp_s7[10] = 0x00;
    resp_s7[11] = 0x00;

    /* Parameter */
    resp_s7[12] = S7_FC_READ_VAR;
    resp_s7[13] = num_items;

    /* Data */
    int out_offset = 14;
    data_offset = 14; /* reset to first item */

    for (int i = 0; i < num_items && data_offset + 12 <= req_len; i++) {
        /* Item spec: type(1) + transport_size(1) + length(2) + db_number(2) + area(1) + address(3) */
        /* Skip item spec type byte */
        uint8_t ts_raw = req[data_offset + 1];
        uint16_t item_len = get_u16_be(&req[data_offset + 2]);
        uint16_t db_num = get_u16_be(&req[data_offset + 4]);
        uint8_t area = req[data_offset + 6];
        uint32_t byte_off = (((uint32_t)req[data_offset + 7] << 16) |
                              ((uint32_t)req[data_offset + 8] << 8) |
                              req[data_offset + 9]) >> 3;

        int byte_count = item_len;
        /* For word/int types, item_len is count of words */
        if (ts_raw == S7_TS_WORD) {
            byte_count = item_len * 2;
        }

        data_offset += 12;

        if (area == S7_AREA_DB && db_num == 1) {
            resp_s7[out_offset++] = 0xFF; /* success */
            int bytes_to_copy = byte_count;
            if (byte_off + bytes_to_copy > DB1_SIZE) {
                bytes_to_copy = DB1_SIZE - (int)byte_off;
                if (bytes_to_copy < 0) bytes_to_copy = 0;
            }
            memcpy(&resp_s7[out_offset], &db1[byte_off], bytes_to_copy);
            out_offset += bytes_to_copy;
        } else {
            resp_s7[out_offset++] = 0x0A; /* object does not exist */
            memset(&resp_s7[out_offset], 0, byte_count);
            out_offset += byte_count;
        }
    }

    return out_offset;
}

/* Build S7 Write Var response */
static int build_write_var_response(const uint8_t *req, int req_len, uint8_t *resp_s7)
{
    if (req_len < 14) return 0;

    uint8_t num_items = req[13];
    int param_len = 2;
    int resp_data_len = 1 + num_items; /* error byte + return code per item */

    /* S7 header (12 bytes) */
    resp_s7[0] = 0x32;
    resp_s7[1] = S7_MSG_TYPE_ACK_DATA;
    put_u16_be(&resp_s7[2], 0x0000);
    put_u16_be(&resp_s7[4], get_u16_be(req + 4));
    put_u16_be(&resp_s7[6], param_len);
    put_u16_be(&resp_s7[8], resp_data_len);
    resp_s7[10] = 0x00;
    resp_s7[11] = 0x00;

    /* Parameter */
    resp_s7[12] = S7_FC_WRITE_VAR;
    resp_s7[13] = num_items;

    /* Parse request items and perform writes */
    int data_offset = 14; /* after header(12) + param(2) */
    int out_offset = 14;

    for (int i = 0; i < num_items && data_offset + 12 <= req_len; i++) {
        uint8_t ts_raw = req[data_offset + 1];
        uint16_t item_len = get_u16_be(&req[data_offset + 2]);
        uint16_t db_num = get_u16_be(&req[data_offset + 4]);
        uint8_t area = req[data_offset + 6];
        uint32_t byte_off = (((uint32_t)req[data_offset + 7] << 16) |
                              ((uint32_t)req[data_offset + 8] << 8) |
                              req[data_offset + 9]) >> 3;

        int byte_count = item_len;
        if (ts_raw == S7_TS_WORD) {
            byte_count = item_len * 2;
        }

        data_offset += 12; /* skip item spec */

        /* Write data follows: return_code(1) + transport_size(1) + length(2) + data */
        if (data_offset + 4 + byte_count <= req_len &&
            area == S7_AREA_DB && db_num == 1)
        {
            int write_bytes = byte_count;
            if (byte_off + write_bytes > DB1_SIZE) {
                write_bytes = DB1_SIZE - (int)byte_off;
                if (write_bytes < 0) write_bytes = 0;
            }
            memcpy(&db1[byte_off], &req[data_offset + 4], write_bytes);

            /* Update board outputs when writing to DB1 bytes 4-14 */
            for (uint32_t b = byte_off; b < byte_off + (uint32_t)write_bytes; b++) {
                if (b >= 4 && b <= 9) {
                    int ch = (int)b - 4;
                    if (ch < 6) board_set_relay(ch, db1[b] != 0);
                } else if (b == 10 || b == 11) {
                    board_set_buzzer(db1[10] != 0 || db1[11] != 0);
                } else if (b >= 12 && b <= 14) {
                    board_set_rgb(db1[12], db1[13], db1[14]);
                }
            }

            resp_s7[out_offset++] = 0xFF; /* success */
        } else if (data_offset + 4 + byte_count <= req_len) {
            resp_s7[out_offset++] = 0x0A; /* object does not exist */
        } else {
            resp_s7[out_offset++] = 0x06; /* fault */
        }

        data_offset += 4 + byte_count; /* skip data header(4) + data */
    }

    return out_offset;
}

/* Process a complete S7 PDU */
static int process_s7_pdu(const uint8_t *pdu, int pdu_len, uint8_t *resp_pdu)
{
    if (pdu_len < 12) return 0;
    if (pdu[0] != 0x32) return 0; /* not S7 protocol */

    uint8_t msg_type = pdu[1];
    uint8_t func_code = pdu[12]; /* first byte of parameter */

    if (msg_type == S7_MSG_TYPE_JOB) {
        switch (func_code) {
        case S7_FC_SETUP_COMM:
            ESP_LOGI(TAG, "S7 Setup Communication request");
            return build_setup_comm_response(pdu, resp_pdu);

        case S7_FC_READ_VAR:
            ESP_LOGI(TAG, "S7 Read Var request (%d items)", pdu[13]);
            return build_read_var_response(pdu, pdu_len, resp_pdu);

        case S7_FC_WRITE_VAR:
            ESP_LOGI(TAG, "S7 Write Var request (%d items)", pdu[13]);
            return build_write_var_response(pdu, pdu_len, resp_pdu);

        default:
            ESP_LOGW(TAG, "S7 unknown function: 0x%02X", func_code);
            /* Build error response */
            memcpy(resp_pdu, pdu, 12);
            resp_pdu[1] = S7_MSG_TYPE_ACK_DATA;
            put_u16_be(&resp_pdu[6], 0);
            put_u16_be(&resp_pdu[8], 0);
            resp_pdu[10] = 0x81;
            resp_pdu[11] = 0x00;
            return 12;
        }
    }

    return 0;
}

static void handle_client(int client_fd)
{
    uint8_t rx_buf[RX_BUFFER_SIZE];
    uint8_t tx_buf[RX_BUFFER_SIZE];

    bool cotp_connected = false;

    while (running) {
        int received = recv(client_fd, rx_buf, RX_BUFFER_SIZE, 0);
        if (received <= 0) {
            if (running) ESP_LOGW(TAG, "client disconnected (fd=%d)", client_fd);
            break;
        }

        if (received < TPKT_HEADER_SIZE) continue;

        /* Parse TPKT header */
        uint16_t tpkt_len = get_u16_be(&rx_buf[2]);
        if (tpkt_len > received) continue;

        if (!cotp_connected) {
            /* Expect COTP Connection Request */
            if (rx_buf[TPKT_HEADER_SIZE + 1] == COTP_CR) {
                ESP_LOGI(TAG, "COTP Connection Request received");
                int resp_len = handle_cotp_cr(rx_buf, received, tx_buf);
                if (resp_len > 0) {
                    send(client_fd, tx_buf, resp_len, 0);
                    cotp_connected = true;
                    ESP_LOGI(TAG, "COTP Connection Confirm sent");
                }
            }
            continue;
        }

        /* COTP connected: expect COTP Data (DT) frames */
        uint8_t cotp_type = rx_buf[TPKT_HEADER_SIZE + 1];
        if (cotp_type != COTP_DT) {
            ESP_LOGW(TAG, "Unexpected COTP type: 0x%02X", cotp_type);
            continue;
        }

        int cotp_header_len = TPKT_HEADER_SIZE + rx_buf[TPKT_HEADER_SIZE] + 1;
        int s7_len = tpkt_len - cotp_header_len;
        if (s7_len <= 0) continue;

        /* Process S7 PDU */
        int resp_pdu_len = process_s7_pdu(&rx_buf[cotp_header_len], s7_len, &tx_buf[10]);
        if (resp_pdu_len > 0) {
            int hdr_len = build_s7_header(tx_buf, resp_pdu_len);
            /* Move response PDU after header */
            memmove(&tx_buf[hdr_len], &tx_buf[10], resp_pdu_len);
            int total = hdr_len + resp_pdu_len;
            send(client_fd, tx_buf, total, 0);
        }
    }

    close(client_fd);
}

static void data_update_task(void *arg)
{
    (void)arg;
    while (running) {
        counter++;
        put_u16_be(&db1[0], counter);
        put_u16_be(&db1[2], (uint16_t)(esp_random() % 1000));

        /* Read relay states from board */
        for (int i = 0; i < 6; i++) {
            db1[4 + i] = board_get_relay(i) ? 1 : 0;
        }
        /* Buzzer state at bytes 10-11 */
        db1[10] = 0;
        db1[11] = 0;
        /* RGB at bytes 12-14 (read-only reflection, kept as-is) */

        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    vTaskDelete(NULL);
}

static void tcp_server_task(void *arg)
{
    (void)arg;

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

    ESP_LOGI(TAG, "S7 server listening on port %d (rack=%d, slot=%d)",
             TCP_PORT, S7_RACK, S7_SLOT);

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

esp_err_t s7_server_init(void)
{
    memset(db1, 0, sizeof(db1));
    board_init();
    running = true;

    ESP_LOGI(TAG, "S7 Server initialized (port=%d, rack=%d, slot=%d)",
             TCP_PORT, S7_RACK, S7_SLOT);
    return ESP_OK;
}

esp_err_t s7_server_start(void)
{
    xTaskCreate(tcp_server_task, "s7_server", 8192, NULL,
                tskIDLE_PRIORITY + 5, NULL);
    xTaskCreate(data_update_task, "s7_data", 2048, NULL,
                tskIDLE_PRIORITY + 3, NULL);

    ESP_LOGI(TAG, "S7 Server started");
    return ESP_OK;
}
