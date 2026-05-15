#include "dnp3_outstation.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"
#include "lwip/netdb.h"
#include "lwip/inet.h"
#include <string.h>
#include <stdlib.h>
#include <errno.h>

static const char *TAG = "dnp3_outstation";

#define TCP_PORT CONFIG_DNP3_TCP_PORT
#define OUTSTATION_ADDR CONFIG_DNP3_OUTSTATION_ADDR
#define RX_BUFFER_SIZE 1024
#define TX_BUFFER_SIZE 1024

#define DNP3_START_BYTE0 0x05
#define DNP3_START_BYTE1 0x64
#define DNP3_DATA_LINK_FRAME_SIZE 10  /* header size */
#define DNP3_MAX_DATA_BLOCK 250
#define DNP3_CRC_SIZE 2

/* Data link function codes */
#define DL_CONFIRMED_USER_DATA 0x44
#define DL_UNCONFIRMED_USER_DATA 0x40

/* Application function codes */
#define FC_READ 0x01

/* Object group/variation IDs */
#define GROUP_BINARY_INPUT  1
#define GROUP_ANALOG_INPUT  30
#define VARIATION_BI_PACKED 1   /* Packed format */
#define VARIATION_AI_FLOAT  4   /* Short float */
#define VARIATION_AI_FLOAT32 5  /* Double float */

/* Qualifier codes */
#define QUALIFIER_ALL  0x06  /* all points, no prefix */
#define QUALIFIER_UINT8_START_STOP 0x00  /* start/stop with 1-byte indices */
#define QUALIFIER_UINT16_START_STOP 0x01 /* start/stop with 2-byte indices */
#define QUALIFIER_UINT8_COUNT  0x07 /* count with 1-byte prefix */
#define QUALIFIER_NONE 0x00

/* Data points */
static bool binary_inputs[5];
static float analog_inputs[5];
static uint32_t counter = 0;
static bool running = false;

/* ---- DNP3 CRC-16 ---- */
static uint16_t dnp3_crc16(const uint8_t *data, int len) {
    uint16_t crc = 0;
    for (int i = 0; i < len; i++) {
        crc ^= data[i];
        for (int j = 0; j < 8; j++) {
            if (crc & 1) {
                crc = (crc >> 1) ^ 0xA6BC; /* polynomial 0x3D65 reflected */
            } else {
                crc >>= 1;
            }
        }
    }
    return crc;
}

/* ---- Append CRC to data block ---- */
/* DNP3 adds CRC after every 16 bytes of user data + at end */
static int append_crc_blocks(uint8_t *buf, int data_len) {
    int written = 0;
    int offset = 0;
    while (offset < data_len) {
        int block_len = data_len - offset;
        if (block_len > 16) block_len = 16;
        /* CRC is for data block only, not including previous CRCs */
        uint16_t crc = dnp3_crc16(&buf[offset + written], block_len);
        /* Shift remaining data to make room for CRC */
        memmove(&buf[offset + written + block_len + 2],
                &buf[offset + written + block_len],
                data_len - offset - block_len);
        buf[offset + written + block_len] = crc & 0xFF;
        buf[offset + written + block_len + 1] = (crc >> 8) & 0xFF;
        written += 2;
        offset += block_len;
    }
    return data_len + written;
}

/* ---- Read data blocks with CRC checking ---- */
/* DNP3 frames: header(10 bytes, CRC at bytes 8-9) then data blocks of 16 bytes + 2 CRC each */
static int read_with_crc(int fd, uint8_t *buf, int max_len) {
    /* Read header (10 bytes) */
    int r = recv(fd, buf, 10, MSG_WAITALL);
    if (r < 10) return -1;

    /* Verify start bytes */
    if (buf[0] != DNP3_START_BYTE0 || buf[1] != DNP3_START_BYTE1) return -1;

    /* Check header CRC */
    uint16_t hdr_crc = dnp3_crc16(buf, 8);
    uint16_t recv_crc = buf[8] | ((uint16_t)buf[9] << 8);
    if (hdr_crc != recv_crc) {
        ESP_LOGW(TAG, "Header CRC mismatch");
        return -1;
    }

    /* Get data length from header */
    uint8_t data_len = buf[2];
    if (data_len < 5 || data_len > 250) return -1;

    /* Total bytes to read: data_len bytes of user data + CRC blocks (every 16 bytes + 2 CRC) */
    int total_data = data_len;
    int crc_blocks = (total_data + 15) / 16;
    int bytes_to_read = total_data + (crc_blocks * 2);

    if (10 + bytes_to_read > max_len) return -1;

    /* Read data + CRC */
    r = recv(fd, &buf[10], bytes_to_read, MSG_WAITALL);
    if (r < bytes_to_read) return -1;

    /* Strip CRCs from data blocks - copy user data only */
    uint8_t *clean_data = malloc(total_data + 16);
    if (!clean_data) return -1;

    int src = 10;
    int dst = 0;
    int remaining = total_data;
    while (remaining > 0) {
        int block = remaining;
        if (block > 16) block = 16;
        /* Verify block CRC */
        uint16_t block_crc = dnp3_crc16(&buf[src], block);
        uint16_t block_recv = buf[src + block] | ((uint16_t)buf[src + block + 1] << 8);
        if (block_crc != block_recv) {
            ESP_LOGW(TAG, "Data block CRC mismatch at offset %d", dst);
            free(clean_data);
            return -1;
        }
        memcpy(&clean_data[dst], &buf[src], block);
        src += block + 2;
        dst += block;
        remaining -= block;
    }

    /* Copy clean data back into buf after header */
    memcpy(&buf[10], clean_data, total_data);
    free(clean_data);

    /* Return: header(10) + clean data(data_len) */
    return 10 + data_len;
}

/* ---- Build DNP3 response frame ---- */
static int build_response(uint8_t *resp, int app_data_len) {
    /* Total frame = header(10) + app_data + CRC blocks in data */
    /* First build with user data, then insert CRCs */

    /* Set header */
    resp[0] = DNP3_START_BYTE0;
    resp[1] = DNP3_START_BYTE1;
    resp[2] = app_data_len; /* data length */
    resp[3] = 0x44; /* control: DIR=1, PRM=1, FCB=0, FCV=0, FC=0x04 (confirmed user data from outstation) */
    /* Actually control byte: bit7=DIR, bit6=PRM, bit5=FCB, bit4=FCV, bits3-0=FC */
    resp[3] = 0xC4; /* DIR=1, PRM=1, FCB=0, FCV=0, FC=0x04 */
    resp[4] = 0x01; /* destination address low byte (master = 1) */
    resp[5] = 0x00; /* destination high */
    resp[6] = OUTSTATION_ADDR & 0xFF; /* source low */
    resp[7] = (OUTSTATION_ADDR >> 8) & 0xFF; /* source high */

    /* Compute header CRC */
    uint16_t hdr_crc = dnp3_crc16(resp, 8);
    resp[8] = hdr_crc & 0xFF;
    resp[9] = (hdr_crc >> 8) & 0xFF;

    /* Insert CRC blocks in the data portion */
    int total = append_crc_blocks(&resp[10], app_data_len);
    return 10 + total;
}

/* ---- Handle Read request ---- */
static int handle_read_request(const uint8_t *req_data, int req_data_len,
                                uint8_t *resp_app, int resp_max) {
    int rp = 0;
    int wp = 0;

    /* Application layer request: AC byte + FC + [Object headers] */
    if (rp >= req_data_len) return 0;
    uint8_t ac = req_data[rp++]; /* application control */
    uint8_t fc = req_data[rp++]; /* function code */

    (void)ac;

    if (fc != FC_READ) {
        ESP_LOGW(TAG, "Unhandled function code: 0x%02X", fc);
        return 0;
    }

    /* Response AC byte: FIR=1, FIN=1, CON=0, UNS=0, SEQ=same as request */
    uint8_t seq = ac & 0x0F;
    resp_app[wp++] = 0xC0 | seq; /* FIR|FIN|CON=0, SEQ */

    /* Response FC */
    resp_app[wp++] = FC_READ;

    /* Internal Indications (IIN) */
    resp_app[wp++] = 0x00; /* IIN byte 1 */
    resp_app[wp++] = 0x00; /* IIN byte 2 */

    /* Process object headers in request */
    while (rp + 2 < req_data_len && wp < resp_max - 16) {
        uint8_t group = req_data[rp++];
        uint8_t variation = req_data[rp++];
        uint8_t qualifier = req_data[rp++];

        ESP_LOGI(TAG, "Read: group=%u var=%u qual=0x%02X", group, variation, qualifier);

        if (group == GROUP_BINARY_INPUT && (variation == VARIATION_BI_PACKED || variation == 0)) {
            /* Binary Input, variation 1 (packed) */
            resp_app[wp++] = GROUP_BINARY_INPUT;
            resp_app[wp++] = VARIATION_BI_PACKED;
            resp_app[wp++] = QUALIFIER_ALL; /* all points, no prefix */
            /* Packed format: 1 bit per point, 5 points packed in 1 byte */
            uint8_t packed = 0;
            for (int i = 0; i < 5; i++) {
                if (binary_inputs[i]) packed |= (1 << i);
            }
            resp_app[wp++] = packed;
        } else if (group == GROUP_ANALOG_INPUT && (variation == VARIATION_AI_FLOAT || variation == 0)) {
            /* Analog Input, variation 4 (short float) */
            resp_app[wp++] = GROUP_ANALOG_INPUT;
            resp_app[wp++] = VARIATION_AI_FLOAT;
            resp_app[wp++] = QUALIFIER_ALL; /* all points */
            /* Each value: flag(1) + float(4) = 5 bytes */
            for (int i = 0; i < 5 && wp + 5 < resp_max; i++) {
                resp_app[wp++] = 0x01; /* flags: online */
                memcpy(&resp_app[wp], &analog_inputs[i], 4);
                wp += 4;
            }
        } else if (group == 60 && variation == 1) {
            /* Class 0 data - return all */
            /* Binary inputs */
            if (wp + 4 < resp_max) {
                resp_app[wp++] = GROUP_BINARY_INPUT;
                resp_app[wp++] = VARIATION_BI_PACKED;
                resp_app[wp++] = QUALIFIER_ALL;
                uint8_t packed = 0;
                for (int i = 0; i < 5; i++) {
                    if (binary_inputs[i]) packed |= (1 << i);
                }
                resp_app[wp++] = packed;
            }
            /* Analog inputs */
            if (wp + 27 < resp_max) {
                resp_app[wp++] = GROUP_ANALOG_INPUT;
                resp_app[wp++] = VARIATION_AI_FLOAT;
                resp_app[wp++] = QUALIFIER_ALL;
                for (int i = 0; i < 5 && wp + 5 < resp_max; i++) {
                    resp_app[wp++] = 0x01; /* flags */
                    memcpy(&resp_app[wp], &analog_inputs[i], 4);
                    wp += 4;
                }
            }
        } else {
            /* Unknown group/variation - skip */
            /* For qualifier 0x06 (all), no prefix bytes to skip */
            if (qualifier == QUALIFIER_UINT8_START_STOP) {
                rp += 2; /* start + stop */
            } else if (qualifier == QUALIFIER_UINT16_START_STOP) {
                rp += 4;
            }
        }
    }

    return wp;
}

/* ---- Process DNP3 frame ---- */
static int process_frame(const uint8_t *frame, int frame_len, uint8_t *resp) {
    if (frame_len < 12) return 0; /* header(10) + min app data(2) */

    /* Check destination = our address (or broadcast) */
    uint8_t ctrl = frame[3];
    bool from_master = (ctrl & 0x40) != 0; /* PRM bit */

    if (!from_master) return 0; /* Ignore outstation messages */

    uint8_t dest_low = frame[4];
    uint8_t dest_high = frame[5];
    uint16_t dest_addr = dest_low | ((uint16_t)dest_high << 8);

    /* Accept if destination matches our address or is broadcast */
    if (dest_addr != OUTSTATION_ADDR && dest_addr != 0xFFFF) return 0;

    uint8_t src_low = frame[6];
    uint8_t src_high = frame[7];
    uint16_t src_addr = src_low | ((uint16_t)src_high << 8);

    uint8_t func_code = ctrl & 0x0F;

    /* Application data starts at byte 10 (after header, CRCs stripped) */
    const uint8_t *app_data = &frame[10];
    int app_data_len = frame_len - 10;

    ESP_LOGI(TAG, "Frame: ctrl=0x%02X func=%u src=%u dest=%u app_len=%d",
             ctrl, func_code, src_addr, dest_addr, app_data_len);

    if (func_code == DL_CONFIRMED_USER_DATA && app_data_len >= 2) {
        /* Build response frame */
        uint8_t resp_app[512];
        int resp_app_len = handle_read_request(app_data, app_data_len, resp_app, sizeof(resp_app));

        if (resp_app_len > 0) {
            /* Set response header: DIR=1(outstation->master), PRM=1, FCV=0, FC=confirm(0x00) or response data */
            /* Actually for response: DIR=1, PRM=0 (from outstation) */
            /* Control: DIR=1, PRM=0, FCB=0, FCV=0, DFC=0, FC=0x00 (ACK) */
            /* Wait - for outstation response with data: DIR=1, PRM=1, FC=confirmed user data(0x04) */
            /* build_response sets the header, we provide app data starting at resp[10] */
            memcpy(&resp[10], resp_app, resp_app_len);
            return build_response(resp, resp_app_len);
        }
    }

    return 0;
}

/* ---- Client handler ---- */
static void handle_client(int client_fd) {
    uint8_t *rx_buf = malloc(RX_BUFFER_SIZE);
    uint8_t *tx_buf = malloc(TX_BUFFER_SIZE);
    if (!rx_buf || !tx_buf) {
        ESP_LOGE(TAG, "alloc failed");
        free(rx_buf); free(tx_buf);
        close(client_fd);
        return;
    }

    while (running) {
        int frame_len = read_with_crc(client_fd, rx_buf, RX_BUFFER_SIZE);
        if (frame_len <= 0) {
            ESP_LOGW(TAG, "client disconnected or frame error");
            break;
        }

        int resp_len = process_frame(rx_buf, frame_len, tx_buf);
        if (resp_len > 0) {
            send(client_fd, tx_buf, resp_len, 0);
        }
    }

    free(rx_buf);
    free(tx_buf);
    close(client_fd);
}

/* ---- Data update task ---- */
static void data_update_task(void *arg) {
    while (running) {
        counter++;
        /* Binary inputs: random toggle every 2s */
        for (int i = 0; i < 5; i++) {
            if (esp_random() % 2) binary_inputs[i] = !binary_inputs[i];
        }

        /* Analog inputs: update every 1s */
        analog_inputs[0] = 20.0f + ((float)(esp_random() % 15000) / 1000.0f); /* temperature */
        analog_inputs[1] = 40.0f + ((float)(esp_random() % 40000) / 1000.0f); /* humidity */
        analog_inputs[2] = (float)counter; /* counter */
        analog_inputs[3] = (float)(esp_random() % 1000); /* random 0-1000 */
        analog_inputs[4] = (float)(esp_random() % 100);  /* random 0-100 */

        vTaskDelay(pdMS_TO_TICKS(1000));
    }
    vTaskDelete(NULL);
}

/* ---- TCP server task ---- */
static void tcp_server_task(void *arg) {
    struct sockaddr_in dest_addr;
    dest_addr.sin_addr.s_addr = htonl(INADDR_ANY);
    dest_addr.sin_family = AF_INET;
    dest_addr.sin_port = htons(TCP_PORT);

    int listen_fd = socket(AF_INET, SOCK_STREAM, IPPROTO_IP);
    if (listen_fd < 0) {
        ESP_LOGE(TAG, "socket create failed: errno %d", errno);
        vTaskDelete(NULL);
        return;
    }

    int opt = 1;
    setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));

    if (bind(listen_fd, (struct sockaddr *)&dest_addr, sizeof(dest_addr)) != 0) {
        ESP_LOGE(TAG, "bind failed: errno %d", errno);
        close(listen_fd);
        vTaskDelete(NULL);
        return;
    }

    if (listen(listen_fd, 1) != 0) {
        ESP_LOGE(TAG, "listen failed: errno %d", errno);
        close(listen_fd);
        vTaskDelete(NULL);
        return;
    }

    ESP_LOGI(TAG, "DNP3 Outstation listening on port %d", TCP_PORT);

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

/* ---- Public API ---- */
esp_err_t dnp3_outstation_init(void) {
    memset(binary_inputs, 0, sizeof(binary_inputs));
    memset(analog_inputs, 0, sizeof(analog_inputs));
    running = true;

    ESP_LOGI(TAG, "DNP3 Outstation initialized (port=%d, addr=%d)",
             TCP_PORT, OUTSTATION_ADDR);
    return ESP_OK;
}

esp_err_t dnp3_outstation_start(void) {
    xTaskCreate(tcp_server_task, "dnp3_srv", 8192, NULL,
                tskIDLE_PRIORITY + 5, NULL);
    xTaskCreate(data_update_task, "dnp3_data", 3072, NULL,
                tskIDLE_PRIORITY + 3, NULL);
    return ESP_OK;
}
