#include "enip_server.h"
#include "esp_log.h"
#include "esp_random.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"
#include "lwip/inet.h"
#include <string.h>
#include <stdlib.h>
#include <errno.h>

static const char *TAG = "enip_server";

#define TCP_PORT CONFIG_ENIP_SERVER_PORT
#define RX_BUFFER_SIZE 1024
#define MAX_CLIENTS 5

/* Encapsulation commands */
#define ENIP_CMD_NOP                0x0001
#define ENIP_CMD_LIST_IDENTITY      0x0063
#define ENIP_CMD_LIST_INTERFACES    0x0064
#define ENIP_CMD_REGISTER_SESSION   0x0065
#define ENIP_CMD_UNREGISTER_SESSION 0x0066
#define ENIP_CMD_SEND_RR_DATA       0x006F
#define ENIP_CMD_SEND_UNIT_DATA     0x0070

/* CIP commands embedded in ForwardOpen/ForwardClose */
#define CIP_FORWARD_OPEN    0x54
#define CIP_FORWARD_CLOSE   0x4E
#define CIP_GET_ATTRIBUTE_ALL 0x01

/* Encapsulation header is 24 bytes */
#define ENIP_HEADER_SIZE 24

/* Device identity */
#define VENDOR_ID       0x1234
#define DEVICE_TYPE     0x000E
#define PRODUCT_CODE    0x0001
#define REVISION_MAJOR  1
#define REVISION_MINOR  1
#define STATUS          0x0000
#define SERIAL_NUMBER   0x00000001UL
#define PRODUCT_NAME    "ESP32 ENIP Tester"

static uint32_t next_session_handle = 0x00000001;
static bool running = false;

static void put_u16_le(uint8_t *p, uint16_t v)
{
    p[0] = v & 0xFF;
    p[1] = (v >> 8) & 0xFF;
}

static void put_u32_le(uint8_t *p, uint32_t v)
{
    p[0] = v & 0xFF;
    p[1] = (v >> 8) & 0xFF;
    p[2] = (v >> 16) & 0xFF;
    p[3] = (v >> 24) & 0xFF;
}

static uint16_t get_u16_le(const uint8_t *p)
{
    return (uint16_t)p[0] | ((uint16_t)p[1] << 8);
}

static uint32_t get_u32_le(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

/* Build encapsulation header into buf, returns pointer to data area */
static int build_enip_header(uint8_t *buf, uint16_t command, uint16_t data_len,
                              uint32_t session_handle, uint32_t status)
{
    put_u16_le(&buf[0], command);           /* command */
    put_u16_le(&buf[2], data_len);          /* length */
    put_u32_le(&buf[4], session_handle);    /* session handle */
    put_u32_le(&buf[8], status);            /* status */
    /* sender context: copy 8 bytes of zero */
    memset(&buf[12], 0, 8);
    put_u32_le(&buf[20], 0);               /* options */
    return ENIP_HEADER_SIZE;
}

/* NOP: no response */
static int handle_nop(const uint8_t *req, int req_len, uint8_t *resp)
{
    (void)req;
    (void)req_len;
    (void)resp;
    return 0;
}

/* ListIdentity: return device identity */
static int handle_list_identity(const uint8_t *req, int req_len, uint8_t *resp)
{
    (void)req;
    (void)req_len;

    /* Item count = 1 */
    int data_offset = ENIP_HEADER_SIZE;
    put_u16_le(&resp[data_offset], 0x0000);    /* item type: NULL (wrapped) -> use CIP Identity */
    put_u16_le(&resp[data_offset + 2], 0x0000); /* will fix below */

    /* Actually, standard ListIdentity response uses item type 0x000C (SocketAddress)
       and 0x000C for each identity item. Let's use the common simplified format. */
    /* Item: type 0x000C (Communications), then identity data */

    /* Common approach: item type = NULL (0x0000), but most tools expect 0x000C */
    /* Use simplified: just identity item */
    put_u16_le(&resp[data_offset], 0x000C);    /* item type: Communications (ListIdentity) */

    /* Build identity data */
    int id_start = data_offset + 4; /* after item type + item length (placeholder) */

    put_u16_le(&resp[id_start], 0x01);         /* encap protocol version */
    put_u16_le(&resp[id_start + 2], 0x0000);   /* sock addr family (placeholder) */
    put_u16_le(&resp[id_start + 4], TCP_PORT);  /* port */
    memset(&resp[id_start + 6], 0, 8);          /* IP address (0 = no discovery addr) */
    memset(&resp[id_start + 14], 0, 8);         /* padding/zero */

    /* Vendor ID */
    put_u16_le(&resp[id_start + 22], VENDOR_ID);
    /* Device Type */
    put_u16_le(&resp[id_start + 24], DEVICE_TYPE);
    /* Product Code */
    put_u16_le(&resp[id_start + 26], PRODUCT_CODE);
    /* Revision */
    resp[id_start + 28] = REVISION_MAJOR;
    resp[id_start + 29] = REVISION_MINOR;
    /* Status */
    put_u16_le(&resp[id_start + 30], STATUS);
    /* Serial Number */
    put_u32_le(&resp[id_start + 32], SERIAL_NUMBER);
    /* Product Name Length + Name */
    uint8_t name_len = (uint8_t)strlen(PRODUCT_NAME);
    resp[id_start + 36] = name_len;
    memcpy(&resp[id_start + 37], PRODUCT_NAME, name_len);
    /* State */
    resp[id_start + 37 + name_len] = 0x03; /* established */

    int id_data_len = 38 + name_len;
    put_u16_le(&resp[data_offset + 2], id_data_len); /* item length */

    int total_data = 4 + id_data_len;
    build_enip_header(resp, ENIP_CMD_LIST_IDENTITY, total_data, 0, 0);
    return ENIP_HEADER_SIZE + total_data;
}

/* RegisterSession: allocate a session handle */
static int handle_register_session(const uint8_t *req, int req_len, uint8_t *resp)
{
    (void)req;
    (void)req_len;

    uint32_t handle = next_session_handle++;

    /* Response data: protocol version(4) + options(4) = 4 bytes */
    int data_offset = ENIP_HEADER_SIZE;
    put_u16_le(&resp[data_offset], 0x0001);     /* protocol version */
    put_u16_le(&resp[data_offset + 2], 0x0000); /* options */

    build_enip_header(resp, ENIP_CMD_REGISTER_SESSION, 4, handle, 0);
    ESP_LOGI(TAG, "Session registered, handle=0x%08lX", (unsigned long)handle);
    return ENIP_HEADER_SIZE + 4;
}

/* UnregisterSession: no response body */
static int handle_unregister_session(const uint8_t *req, int req_len, uint8_t *resp)
{
    (void)req;
    (void)req_len;
    (void)resp;

    ESP_LOGI(TAG, "Session unregistered");
    return 0; /* no response */
}

/* ForwardOpen (inside SendRRData) */
static int handle_forward_open(const uint8_t *req_data, int data_len, uint8_t *resp_data)
{
    (void)data_len;

    /* CIP response: service(1) + reserved(1) + status(1) + ext_status(1) + data */
    resp_data[0] = CIP_FORWARD_OPEN | 0x80; /* response bit */
    resp_data[1] = 0x00; /* reserved */
    resp_data[2] = 0x00; /* status: success */
    resp_data[3] = 0x00; /* ext status words: 0 */

    /* Open connection parameters from request (pass through key values) */
    /* OT connection ID, T->O connection ID, connection serial, vendor, originator serial */
    /* Skip path: path_size is at req_data+1, path follows */
    uint8_t path_size = req_data[1]; /* in 16-bit words */
    int path_bytes = path_size * 2;
    int params_offset = 2 + path_bytes;

    if (params_offset + 36 <= data_len) {
        /* ForwardOpen has fixed params after path:
           priority(1) + timeout_ticks(1) + O->T connection ID(4) + T->O connection ID(4)
           + O->T RPI(4) + O->T params(2) + T->O RPI(4) + T->O params(2)
           + transport_class(1) + path... */
        uint32_t ot_conn_id = get_u32_le(&req_data[params_offset + 2]);
        (void)ot_conn_id;

        /* Response: O->T connection ID(4) + T->O connection ID(4) + connection serial(2)
           + vendor ID(2) + originator serial(4) + O->T RPI(4) + O->T params(2)
           + T->O RPI(4) + T->O params(2) + transport(1) + reserved(3) */
        memcpy(&resp_data[4], &req_data[params_offset + 2], 4);  /* O->T conn ID */
        memcpy(&resp_data[8], &req_data[params_offset + 6], 4);  /* T->O conn ID */
        memcpy(&resp_data[12], &req_data[params_offset + 10], 2); /* conn serial */
        memcpy(&resp_data[14], &req_data[params_offset + 12], 2); /* vendor ID */
        memcpy(&resp_data[16], &req_data[params_offset + 14], 4); /* originator serial */
        memcpy(&resp_data[20], &req_data[params_offset + 18], 4); /* O->T RPI */
        memcpy(&resp_data[24], &req_data[params_offset + 22], 2); /* O->T params */
        memcpy(&resp_data[26], &req_data[params_offset + 24], 4); /* T->O RPI */
        memcpy(&resp_data[30], &req_data[params_offset + 28], 2); /* T->O params */
        resp_data[32] = req_data[params_offset + 30];              /* transport */
        memset(&resp_data[33], 0, 3);                              /* reserved */

        return 36;
    }

    /* Minimal response if request was too short */
    return 4;
}

/* ForwardClose (inside SendRRData) */
static int handle_forward_close(const uint8_t *req_data, int data_len, uint8_t *resp_data)
{
    (void)data_len;

    resp_data[0] = CIP_FORWARD_CLOSE | 0x80;
    resp_data[1] = 0x00;
    resp_data[2] = 0x00; /* success */
    resp_data[3] = 0x00;

    /* Echo back connection serial, vendor, originator serial */
    uint8_t path_size = req_data[1];
    int path_bytes = path_size * 2;
    int params_offset = 2 + path_bytes;

    if (params_offset + 8 <= data_len) {
        memcpy(&resp_data[4], &req_data[params_offset + 2], 2); /* conn serial */
        memcpy(&resp_data[6], &req_data[params_offset + 4], 2); /* vendor */
        memcpy(&resp_data[8], &req_data[params_offset + 6], 4); /* originator serial */
        return 12;
    }

    return 4;
}

/* GetAttributeAll (inside SendRRData) */
static int handle_get_attribute_all(const uint8_t *req_data, int data_len, uint8_t *resp_data)
{
    (void)req_data;
    (void)data_len;

    resp_data[0] = CIP_GET_ATTRIBUTE_ALL | 0x80;
    resp_data[1] = 0x00;
    resp_data[2] = 0x00;
    resp_data[3] = 0x00;

    /* Attribute data: identity info */
    put_u16_le(&resp_data[4], VENDOR_ID);
    put_u16_le(&resp_data[6], DEVICE_TYPE);
    put_u16_le(&resp_data[8], PRODUCT_CODE);
    resp_data[10] = REVISION_MAJOR;
    resp_data[11] = REVISION_MINOR;
    put_u16_le(&resp_data[12], STATUS);
    put_u32_le(&resp_data[14], SERIAL_NUMBER);

    return 18;
}

/* Handle SendRRData (ForwardOpen, ForwardClose, GetAttributeAll) */
static int handle_send_rr_data(const uint8_t *req, int req_len, uint8_t *resp)
{
    if (req_len < ENIP_HEADER_SIZE + 6) {
        return 0;
    }

    int data_offset = ENIP_HEADER_SIZE;
    /* Skip encapsulation data: interface handle(4) + timeout(2) */
    int cip_offset = data_offset + 6;

    /* Items: item type(2) + item length(2) for each item */
    /* Typical: NULL item (type 0x0000, len 0) then UNCONNECTED data item (type 0x00B2) */
    if (cip_offset + 4 > req_len) return 0;

    uint16_t addr_item_type = get_u16_le(&req[cip_offset]);
    uint16_t addr_item_len = get_u16_le(&req[cip_offset + 2]);

    if (addr_item_type == 0x0000) {
        /* NULL address item, next is data item */
        cip_offset += 4 + addr_item_len;
    } else {
        cip_offset += 4 + addr_item_len;
    }

    if (cip_offset + 4 > req_len) return 0;

    uint16_t data_item_type = get_u16_le(&req[cip_offset]);
    uint16_t data_item_len = get_u16_le(&req[cip_offset + 2]);

    if (data_item_type != 0x00B2) {
        /* Not unconnected data */
        return 0;
    }

    int cip_data_offset = cip_offset + 4;
    if (cip_data_offset >= req_len) return 0;

    uint8_t cip_service = req[cip_data_offset];

    /* Build response: encapsulation header + interface(4) + timeout(2) +
       NULL addr item(4) + data item header(4) + CIP response */
    int resp_data_offset = ENIP_HEADER_SIZE + 6; /* interface + timeout */
    /* NULL address item */
    put_u16_le(&resp[resp_data_offset], 0x0000);
    put_u16_le(&resp[resp_data_offset + 2], 0x0000);
    /* Data item header (placeholder) */
    put_u16_le(&resp[resp_data_offset + 4], 0x00B2);
    put_u16_le(&resp[resp_data_offset + 6], 0x0000); /* length placeholder */

    int cip_resp_len = 0;
    switch (cip_service) {
    case CIP_FORWARD_OPEN:
        ESP_LOGI(TAG, "ForwardOpen request");
        cip_resp_len = handle_forward_open(&req[cip_data_offset],
                                            data_item_len, &resp[resp_data_offset + 8]);
        break;

    case CIP_FORWARD_CLOSE:
        ESP_LOGI(TAG, "ForwardClose request");
        cip_resp_len = handle_forward_close(&req[cip_data_offset],
                                             data_item_len, &resp[resp_data_offset + 8]);
        break;

    case CIP_GET_ATTRIBUTE_ALL:
        ESP_LOGI(TAG, "GetAttributeAll request");
        cip_resp_len = handle_get_attribute_all(&req[cip_data_offset],
                                                 data_item_len, &resp[resp_data_offset + 8]);
        break;

    default:
        ESP_LOGW(TAG, "Unknown CIP service: 0x%02X", cip_service);
        /* Return CIP error */
        resp[resp_data_offset + 8] = cip_service | 0x80;
        resp[resp_data_offset + 9] = 0x00;
        resp[resp_data_offset + 10] = 0x01; /* unsupported service */
        resp[resp_data_offset + 11] = 0x00;
        cip_resp_len = 4;
        break;
    }

    /* Fix data item length */
    put_u16_le(&resp[resp_data_offset + 6], cip_resp_len);

    uint32_t session = get_u32_le(&req[4]);
    int total_data = 6 + 4 + 4 + cip_resp_len; /* interface+timeout + addr_item + data_item + cip */
    build_enip_header(resp, ENIP_CMD_SEND_RR_DATA, total_data, session, 0);

    return ENIP_HEADER_SIZE + total_data;
}

static int process_request(const uint8_t *req, int req_len, uint8_t *resp)
{
    if (req_len < ENIP_HEADER_SIZE) return 0;

    uint16_t command = get_u16_le(&req[0]);
    uint16_t length = get_u16_le(&req[2]);

    if (ENIP_HEADER_SIZE + length > req_len) {
        /* Wait for more data */
        return 0;
    }

    switch (command) {
    case ENIP_CMD_NOP:
        return handle_nop(req, req_len, resp);

    case ENIP_CMD_LIST_IDENTITY:
        return handle_list_identity(req, req_len, resp);

    case ENIP_CMD_REGISTER_SESSION:
        return handle_register_session(req, req_len, resp);

    case ENIP_CMD_UNREGISTER_SESSION:
        return handle_unregister_session(req, req_len, resp);

    case ENIP_CMD_SEND_RR_DATA:
        return handle_send_rr_data(req, req_len, resp);

    case ENIP_CMD_SEND_UNIT_DATA:
        /* Similar to SendRRData but for connected messages - simplified response */
        return handle_send_rr_data(req, req_len, resp);

    default:
        ESP_LOGW(TAG, "Unknown ENIP command: 0x%04X", command);
        /* Return error response */
        build_enip_header(resp, command, 0, 0, 0x00000001);
        return ENIP_HEADER_SIZE;
    }
}

static void handle_client(int client_fd)
{
    uint8_t rx_buf[RX_BUFFER_SIZE];
    uint8_t tx_buf[RX_BUFFER_SIZE];
    int rx_used = 0;

    while (running) {
        int space = RX_BUFFER_SIZE - rx_used;
        if (space <= 0) {
            rx_used = 0;
            space = RX_BUFFER_SIZE;
        }

        int received = recv(client_fd, &rx_buf[rx_used], space, 0);
        if (received <= 0) {
            if (running) ESP_LOGW(TAG, "client disconnected (fd=%d)", client_fd);
            break;
        }

        rx_used += received;

        /* Try to parse and process complete messages */
        while (rx_used >= ENIP_HEADER_SIZE) {
            uint16_t pkt_len = get_u16_le(&rx_buf[2]);
            int total = ENIP_HEADER_SIZE + pkt_len;

            if (total > rx_used) {
                break; /* need more data */
            }

            int resp_len = process_request(rx_buf, total, tx_buf);
            if (resp_len > 0) {
                send(client_fd, tx_buf, resp_len, 0);
            }

            /* Shift remaining data */
            int remaining = rx_used - total;
            if (remaining > 0) {
                memmove(rx_buf, &rx_buf[total], remaining);
            }
            rx_used = remaining;
        }
    }

    close(client_fd);
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

    ESP_LOGI(TAG, "EtherNet/IP server listening on port %d", TCP_PORT);

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

esp_err_t enip_server_init(void)
{
    running = true;

    ESP_LOGI(TAG, "EtherNet/IP Server initialized (port=%d)", TCP_PORT);
    return ESP_OK;
}

esp_err_t enip_server_start(void)
{
    xTaskCreate(tcp_server_task, "enip_server", 8192, NULL,
                tskIDLE_PRIORITY + 5, NULL);

    ESP_LOGI(TAG, "EtherNet/IP Server started");
    return ESP_OK;
}
