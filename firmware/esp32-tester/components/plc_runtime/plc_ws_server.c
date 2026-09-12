#include "plc_ws_server.h"
#include "plc_flatbuf.h"
#include "plc_runtime.h"
#include "plc_bytecode.h"
#include "esp_log.h"
#include "esp_http_server.h"
#include "esp_heap_caps.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

#ifdef CONFIG_HTTPD_WS_SUPPORT
#include <esp_http_server.h>
#endif

static const char *TAG = "plc_ws";

// WebSocket connection tracking
#define PLC_WS_MAX_CLIENTS  4

static struct {
    httpd_handle_t   server;
    httpd_handle_t   external_server;  // Set by main app
    int              client_fds[PLC_WS_MAX_CLIENTS];
    volatile uint8_t client_count;
    uint8_t         *encode_buf;
    bool             initialized;
} s_ws;

#ifdef CONFIG_HTTPD_WS_SUPPORT

// Forward declarations
static esp_err_t ws_handler(httpd_req_t *req);
static esp_err_t deploy_post_handler(httpd_req_t *req);
static void handle_ws_command(const uint8_t *data, uint32_t len, httpd_handle_t server, int fd);
static esp_err_t health_get_handler(httpd_req_t *req);
static esp_err_t state_get_handler(httpd_req_t *req);
static esp_err_t variables_get_handler(httpd_req_t *req);
static esp_err_t variable_write_handler(httpd_req_t *req);

// --- HTTP POST /api/deploy ---

static esp_err_t deploy_post_handler(httpd_req_t *req) {
    int content_len = req->content_len;
    if (content_len <= 0 || content_len > 65536) {
        ESP_LOGE(TAG, "invalid content length: %d", content_len);
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Invalid content length");
        return ESP_FAIL;
    }

    uint8_t *buf = heap_caps_malloc(content_len, MALLOC_CAP_8BIT);
    if (!buf) {
        ESP_LOGE(TAG, "failed to alloc %d bytes for bytecode", content_len);
        httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "Out of memory");
        return ESP_FAIL;
    }

    int received = httpd_req_recv(req, (char *)buf, content_len);
    if (received != content_len) {
        ESP_LOGE(TAG, "received %d of %d bytes", received, content_len);
        free(buf);
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Incomplete data");
        return ESP_FAIL;
    }

    esp_err_t ret = plc_runtime_load_bytecode(buf, received);
    free(buf);

    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "bytecode load failed: %s", esp_err_to_name(ret));
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Bytecode validation failed");
        return ESP_FAIL;
    }

    ESP_LOGI(TAG, "bytecode deployed successfully (%d bytes)", content_len);

    httpd_resp_set_type(req, "application/json");
    const char *resp = "{\"status\":\"ok\",\"message\":\"Bytecode deployed\"}";
    httpd_resp_send(req, resp, strlen(resp));

    plc_ws_push_variables(plc_runtime_get_vm());

    return ESP_OK;
}

// --- WebSocket /ws/plc ---

static esp_err_t ws_handler(httpd_req_t *req) {
    if (req->method == HTTP_GET) {
        ESP_LOGI(TAG, "WS client connected, fd=%d", httpd_req_to_sockfd(req));

        if (s_ws.client_count < PLC_WS_MAX_CLIENTS) {
            s_ws.client_fds[s_ws.client_count++] = httpd_req_to_sockfd(req);
        } else {
            ESP_LOGW(TAG, "max WS clients reached, replacing slot 0");
            s_ws.client_fds[0] = httpd_req_to_sockfd(req);
        }
        return ESP_OK;
    }

    httpd_ws_frame_t ws_pkt;
    memset(&ws_pkt, 0, sizeof(ws_pkt));
    ws_pkt.type = HTTPD_WS_TYPE_BINARY;

    esp_err_t ret = httpd_ws_recv_frame(req, &ws_pkt, 0);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "WS recv frame len failed: %s", esp_err_to_name(ret));
        return ret;
    }

    if (ws_pkt.len > 0 && ws_pkt.len < 4096) {
        uint8_t *buf = heap_caps_malloc(ws_pkt.len, MALLOC_CAP_8BIT);
        if (buf) {
            ws_pkt.payload = buf;
            ret = httpd_ws_recv_frame(req, &ws_pkt, ws_pkt.len);
            if (ret == ESP_OK) {
                handle_ws_command(buf, ws_pkt.len, req->handle, httpd_req_to_sockfd(req));
            }
            free(buf);
        }
    }

    return ESP_OK;
}

static void handle_ws_command(const uint8_t *data, uint32_t len, httpd_handle_t server, int fd) {
    if (len < 1) return;

    uint8_t msg_type = data[0];
    char name_buf[PLC_MAX_NAME_LEN];
    plc_value_t val;

    switch (msg_type) {
        case PLC_WS_MSG_FORCE: {
            esp_err_t ret = plc_flatbuf_decode_force(data, len, name_buf, &val);
            if (ret == ESP_OK) {
                ret = plc_runtime_force_variable_by_name(name_buf, &val);
                if (ret == ESP_OK) {
                    ESP_LOGI(TAG, "forced var '%s'", name_buf);
                }
            }
            break;
        }

        case PLC_WS_MSG_UNFORCE: {
            esp_err_t ret = plc_flatbuf_decode_unforce(data, len, name_buf);
            if (ret == ESP_OK) {
                ret = plc_runtime_unforce_variable_by_name(name_buf);
                if (ret == ESP_OK) {
                    ESP_LOGI(TAG, "unforced var '%s'", name_buf);
                }
            }
            break;
        }

        case PLC_WS_MSG_START: {
            uint32_t cycle_time_ms = 10;
            plc_flatbuf_decode_start(data, len, &cycle_time_ms);
            plc_runtime_start(cycle_time_ms);
            ESP_LOGI(TAG, "start command, cycle=%" PRIu32 "ms", cycle_time_ms);
            break;
        }

        case PLC_WS_MSG_STOP:
            plc_runtime_stop();
            ESP_LOGI(TAG, "stop command");
            break;

        case PLC_WS_MSG_STEP:
            plc_runtime_step();
            ESP_LOGI(TAG, "step command");
            break;

        default:
            ESP_LOGW(TAG, "unknown WS command: 0x%02X", msg_type);
            break;
    }
}

// --- REST API handlers ---

// GET /api/health
static esp_err_t health_get_handler(httpd_req_t *req) {
    httpd_resp_set_type(req, "application/json");
    httpd_resp_sendstr(req, "{\"status\":\"ok\"}");
    return ESP_OK;
}

// GET /api/state
static esp_err_t state_get_handler(httpd_req_t *req) {
    plc_runtime_status_t status = plc_runtime_get_status();

    const char *state_str = "stopped";
    switch (status.state) {
        case PLC_STATE_RUNNING: state_str = "running"; break;
        case PLC_STATE_PAUSED:  state_str = "paused"; break;
        case PLC_STATE_ERROR:   state_str = "error"; break;
        default: break;
    }

    char buf[256];
    int len = snprintf(buf, sizeof(buf),
        "{\"state\":\"%s\",\"cycle_count\":%" PRIu32 ","
        "\"scan_time_us\":%" PRIu32 ",\"max_scan_time_us\":%" PRIu32 ","
        "\"loaded_bundle_crc32\":%" PRIu32 "}",
        state_str, status.cycle_count, status.scan_time_us, status.max_scan_time_us,
        status.loaded_bundle_crc32);

    httpd_resp_set_type(req, "application/json");
    httpd_resp_send(req, buf, len);
    return ESP_OK;
}

// GET /api/variables
static esp_err_t variables_get_handler(httpd_req_t *req) {
    char buf[4096];
    int len = plc_runtime_get_variables_json(buf, sizeof(buf));
    httpd_resp_set_type(req, "application/json");
    httpd_resp_send(req, buf, len);
    return ESP_OK;
}

// POST /api/variables/<name>
static esp_err_t variable_write_handler(httpd_req_t *req) {
    // Extract variable name from URI: /api/variables/<name>
    const char *uri = req->uri;
    const char *name_start = uri + strlen("/api/variables/");
    char var_name[PLC_MAX_NAME_LEN] = {0};
    int i = 0;
    while (*name_start && *name_start != '?' && i < (int)sizeof(var_name) - 1) {
        var_name[i++] = *name_start++;
    }

    if (i == 0) {
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Missing variable name");
        return ESP_FAIL;
    }

    // Read JSON body: {"value": <number>}
    int content_len = req->content_len;
    if (content_len <= 0 || content_len > 256) {
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Invalid body");
        return ESP_FAIL;
    }

    char body[257];
    int received = httpd_req_recv(req, body, content_len);
    if (received != content_len) {
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "Incomplete body");
        return ESP_FAIL;
    }
    body[received] = '\0';

    // Parse value from JSON (simple extraction: find "value" key)
    double value = 0;
    char *val_str = strstr(body, "\"value\"");
    if (val_str) {
        val_str = strchr(val_str + 7, ':');
        if (val_str) {
            value = atof(val_str + 1);
        }
    }

    // Force variable by name
    plc_value_t force_val = {
        .type = PLC_TYPE_DINT,
        .dint_val = (int32_t)value,
    };
    esp_err_t ret = plc_runtime_force_variable_by_name(var_name, &force_val);

    httpd_resp_set_type(req, "application/json");
    if (ret == ESP_OK) {
        httpd_resp_sendstr(req, "{\"success\":true}");
    } else {
        httpd_resp_sendstr(req, "{\"success\":false,\"error\":\"Variable not found\"}");
    }
    return ESP_OK;
}

// --- URI registration ---

static const httpd_uri_t ws_uri = {
    .uri        = "/ws/plc",
    .method     = HTTP_GET,
    .handler    = ws_handler,
    .user_ctx   = NULL,
    .is_websocket = true,
};

static const httpd_uri_t deploy_uri = {
    .uri        = "/api/deploy",
    .method     = HTTP_POST,
    .handler    = deploy_post_handler,
    .user_ctx   = NULL,
};

static const httpd_uri_t health_uri = {
    .uri        = "/api/health",
    .method     = HTTP_GET,
    .handler    = health_get_handler,
    .user_ctx   = NULL,
};

static const httpd_uri_t state_uri = {
    .uri        = "/api/state",
    .method     = HTTP_GET,
    .handler    = state_get_handler,
    .user_ctx   = NULL,
};

static const httpd_uri_t variables_uri = {
    .uri        = "/api/variables",
    .method     = HTTP_GET,
    .handler    = variables_get_handler,
    .user_ctx   = NULL,
};

static const httpd_uri_t variable_write_uri = {
    .uri        = "/api/variables/*",
    .method     = HTTP_POST,
    .handler    = variable_write_handler,
    .user_ctx   = NULL,
};

esp_err_t plc_ws_server_init(httpd_handle_t server) {
    if (!server) {
        ESP_LOGE(TAG, "NULL server handle");
        return ESP_ERR_INVALID_ARG;
    }

    s_ws.server = server;
    s_ws.encode_buf = heap_caps_malloc(PLC_FLATBUF_MAX_FRAME, MALLOC_CAP_8BIT);
    if (!s_ws.encode_buf) {
        ESP_LOGE(TAG, "failed to alloc encode buffer");
        return ESP_ERR_NO_MEM;
    }

    esp_err_t ret = httpd_register_uri_handler(server, &ws_uri);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "failed to register WS handler: %s", esp_err_to_name(ret));
        return ret;
    }

    ret = httpd_register_uri_handler(server, &deploy_uri);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "failed to register deploy handler: %s", esp_err_to_name(ret));
        return ret;
    }

    ret = httpd_register_uri_handler(server, &health_uri);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "failed to register health handler: %s", esp_err_to_name(ret));
        return ret;
    }

    ret = httpd_register_uri_handler(server, &state_uri);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "failed to register state handler: %s", esp_err_to_name(ret));
        return ret;
    }

    ret = httpd_register_uri_handler(server, &variables_uri);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "failed to register variables handler: %s", esp_err_to_name(ret));
        return ret;
    }

    ret = httpd_register_uri_handler(server, &variable_write_uri);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "failed to register variable write handler: %s", esp_err_to_name(ret));
        return ret;
    }

    s_ws.client_count = 0;
    for (int i = 0; i < PLC_WS_MAX_CLIENTS; i++) {
        s_ws.client_fds[i] = -1;
    }
    s_ws.initialized = true;

    ESP_LOGI(TAG, "WS server initialized (ws://*/ws/plc, /api/deploy, /api/health, /api/state, /api/variables)");
    return ESP_OK;
}

esp_err_t plc_ws_push_variables(plc_vm_t *vm) {
    if (!s_ws.initialized || !s_ws.encode_buf || s_ws.client_count == 0) {
        return ESP_OK;
    }

    uint32_t frame_len = 0;
    esp_err_t ret = plc_flatbuf_encode_variables(vm, s_ws.encode_buf, PLC_FLATBUF_MAX_FRAME, &frame_len);
    if (ret != ESP_OK || frame_len == 0) {
        return ret;
    }

    httpd_ws_frame_t ws_pkt;
    memset(&ws_pkt, 0, sizeof(ws_pkt));
    ws_pkt.payload = s_ws.encode_buf;
    ws_pkt.len = frame_len;
    ws_pkt.type = HTTPD_WS_TYPE_BINARY;
    ws_pkt.final = true;

    for (int i = 0; i < s_ws.client_count; i++) {
        if (s_ws.client_fds[i] >= 0) {
            ret = httpd_ws_send_frame_async(s_ws.server, s_ws.client_fds[i], &ws_pkt);
            if (ret != ESP_OK) {
                ESP_LOGD(TAG, "failed to send to fd=%d: %s", s_ws.client_fds[i], esp_err_to_name(ret));
                s_ws.client_fds[i] = -1;
            }
        }
    }

    int write_idx = 0;
    for (int i = 0; i < s_ws.client_count; i++) {
        if (s_ws.client_fds[i] >= 0) {
            s_ws.client_fds[write_idx++] = s_ws.client_fds[i];
        }
    }
    s_ws.client_count = write_idx;

    return ESP_OK;
}

esp_err_t plc_ws_push_status(void) {
    if (!s_ws.initialized || !s_ws.encode_buf || s_ws.client_count == 0) {
        return ESP_OK;
    }

    plc_runtime_status_t status = plc_runtime_get_status();

    uint8_t state_code = 0;
    switch (status.state) {
        case PLC_STATE_RUNNING: state_code = 1; break;
        case PLC_STATE_PAUSED:  state_code = 2; break;
        case PLC_STATE_ERROR:   state_code = 3; break;
        default: break;
    }

    uint32_t frame_len = 0;
    esp_err_t ret = plc_flatbuf_encode_status(
        state_code, (uint32_t)status.cycle_count,
        status.scan_time_us, status.max_scan_time_us,
        status.variable_count, s_ws.encode_buf, PLC_FLATBUF_MAX_FRAME, &frame_len);
    if (ret != ESP_OK || frame_len == 0) {
        return ret;
    }

    httpd_ws_frame_t ws_pkt;
    memset(&ws_pkt, 0, sizeof(ws_pkt));
    ws_pkt.payload = s_ws.encode_buf;
    ws_pkt.len = frame_len;
    ws_pkt.type = HTTPD_WS_TYPE_BINARY;
    ws_pkt.final = true;

    for (int i = 0; i < s_ws.client_count; i++) {
        if (s_ws.client_fds[i] >= 0) {
            ret = httpd_ws_send_frame_async(s_ws.server, s_ws.client_fds[i], &ws_pkt);
            if (ret != ESP_OK) {
                s_ws.client_fds[i] = -1;
            }
        }
    }

    // Compact client list
    int write_idx = 0;
    for (int i = 0; i < s_ws.client_count; i++) {
        if (s_ws.client_fds[i] >= 0) {
            s_ws.client_fds[write_idx++] = s_ws.client_fds[i];
        }
    }
    s_ws.client_count = write_idx;

    return ESP_OK;
}

#else  // !CONFIG_HTTPD_WS_SUPPORT

// Stub implementations when WebSocket support is not enabled

esp_err_t plc_ws_server_init(httpd_handle_t server) {
    ESP_LOGW(TAG, "WebSocket support not enabled (CONFIG_HTTPD_WS_SUPPORT)");
    return ESP_ERR_NOT_SUPPORTED;
}

esp_err_t plc_ws_push_variables(plc_vm_t *vm) {
    return ESP_OK;
}

esp_err_t plc_ws_push_status(void) {
    return ESP_OK;
}

#endif  // CONFIG_HTTPD_WS_SUPPORT

void plc_ws_server_stop(void) {
    s_ws.client_count = 0;
    if (s_ws.encode_buf) {
        free(s_ws.encode_buf);
        s_ws.encode_buf = NULL;
    }
    s_ws.initialized = false;
    ESP_LOGI(TAG, "WS server stopped");
}

bool plc_ws_has_client(void) {
    return s_ws.client_count > 0;
}

void plc_ws_set_http_server(httpd_handle_t server) {
    s_ws.external_server = server;
}
