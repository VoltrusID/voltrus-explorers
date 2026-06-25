#include "protocol_launcher.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_http_server.h"
#include "mdns.h"
#include "esp_ota_ops.h"
#include <string.h>
#include <inttypes.h>

#include "modbus_tcp_slave.h"
#include "modbus_tcp_master.h"
#include "modbus_rtu_slave.h"
#include "my_mqtt_client.h"
#include "s7_server.h"
#include "enip_server.h"
#include "opcua_server.h"
#include "bacnet_device.h"
#include "dnp3_outstation.h"
#include "serial_bridge.h"
#include "plc_runtime.h"
#include "plc_ws_server.h"

static const char *TAG = "protocol_launcher";

// HTTP server for PLC Runtime mode
static httpd_handle_t s_plc_http_server = NULL;

static httpd_handle_t start_http_server(void) {
    httpd_config_t config = HTTPD_DEFAULT_CONFIG();
    config.server_port = 80;
    config.max_uri_handlers = 8;

    httpd_handle_t server = NULL;
    if (httpd_start(&server, &config) == ESP_OK) {
        ESP_LOGI(TAG, "HTTP server started on port 80");
        return server;
    }
    ESP_LOGE(TAG, "failed to start HTTP server");
    return NULL;
}

#ifdef CONFIG_VOLTRUS_PLC_OTA_ENABLE
static esp_err_t firmware_post_handler(httpd_req_t *req) {
    int content_len = req->content_len;
    if (content_len <= 0) {
        httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "No content");
        return ESP_FAIL;
    }

    const esp_partition_t *update_partition = esp_ota_get_next_update_partition(NULL);
    if (!update_partition) {
        httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "No OTA partition");
        return ESP_FAIL;
    }

    esp_ota_handle_t ota_handle;
    esp_err_t err = esp_ota_begin(update_partition, content_len, &ota_handle);
    if (err != ESP_OK) {
        httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "OTA begin failed");
        return ESP_FAIL;
    }

    char buf[4096];
    int remaining = content_len;
    while (remaining > 0) {
        int to_read = remaining > sizeof(buf) ? sizeof(buf) : remaining;
        int received = httpd_req_recv(req, buf, to_read);
        if (received <= 0) {
            esp_ota_abort(ota_handle);
            httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "Receive failed");
            return ESP_FAIL;
        }
        esp_ota_write(ota_handle, buf, received);
        remaining -= received;
    }

    err = esp_ota_end(ota_handle);
    if (err != ESP_OK) {
        httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "OTA end failed");
        return ESP_FAIL;
    }

    err = esp_ota_set_boot_partition(update_partition);
    if (err != ESP_OK) {
        httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "Set boot partition failed");
        return ESP_FAIL;
    }

    httpd_resp_sendstr(req, "{\"success\":true,\"message\":\"Firmware updated, restarting...\"}");
    ESP_LOGI(TAG, "OTA firmware update complete, restarting in 2s...");
    vTaskDelay(pdMS_TO_TICKS(2000));
    esp_restart();
    return ESP_OK;
}
#endif

esp_err_t protocol_launcher_start(esp_netif_t *netif) {
#if CONFIG_ESP_TESTER_PROTOCOL_MODBUS_SLAVE
    ESP_LOGI(TAG, "starting Modbus TCP Slave");
    modbus_tcp_slave_init(netif);
    return modbus_tcp_slave_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_MODBUS_MASTER
    ESP_LOGI(TAG, "starting Modbus TCP Master");
    modbus_tcp_master_init(netif);
    return modbus_tcp_master_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_MODBUS_RTU
    ESP_LOGI(TAG, "starting Modbus RTU Slave");
    modbus_rtu_slave_init();
    return modbus_rtu_slave_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_MQTT
    ESP_LOGI(TAG, "starting MQTT Client");
    mqtt_client_init();
    return mqtt_client_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_S7
    ESP_LOGI(TAG, "starting S7 Server");
    s7_server_init();
    return s7_server_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_ENIP
    ESP_LOGI(TAG, "starting EtherNet/IP Server");
    enip_server_init();
    return enip_server_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_OPCUA
    ESP_LOGI(TAG, "starting OPC UA Server");
    opcua_server_init();
    return opcua_server_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_BACNET
    ESP_LOGI(TAG, "starting BACnet Device");
    bacnet_device_init();
    return bacnet_device_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_DNP3
    ESP_LOGI(TAG, "starting DNP3 Outstation");
    dnp3_outstation_init();
    return dnp3_outstation_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_SERIAL
    ESP_LOGI(TAG, "starting Serial Bridge");
    serial_bridge_init();
    return serial_bridge_start();
#elif CONFIG_ESP_TESTER_PROTOCOL_PLC_RUNTIME
    ESP_LOGI(TAG, "starting PLC Runtime");

    // Initialize PLC runtime
    esp_err_t ret = plc_runtime_init();
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "PLC runtime init failed");
        return ret;
    }

    // Start HTTP server for WebSocket and deploy endpoints
    s_plc_http_server = start_http_server();
    if (!s_plc_http_server) {
        return ESP_FAIL;
    }

    // Register WebSocket and deploy handlers
    ret = plc_ws_server_init(s_plc_http_server);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "PLC WS server init failed");
        return ret;
    }

#ifdef CONFIG_VOLTRUS_PLC_MDNS_ENABLE
    esp_err_t mdns_ret = mdns_init();
    if (mdns_ret == ESP_OK) {
        mdns_hostname_set("voltrus-plc");
        mdns_instance_name_set("Voltrus PLC Runtime");
        mdns_service_add(NULL, "_voltrus-runtime", "_tcp", 80, NULL, 0);
        ESP_LOGI(TAG, "mDNS: _voltrus-runtime._tcp registered on port 80");
    } else {
        ESP_LOGW(TAG, "mDNS init failed: %s", esp_err_to_name(mdns_ret));
    }
#endif

#ifdef CONFIG_VOLTRUS_PLC_OTA_ENABLE
    const httpd_uri_t firmware_uri = {
        .uri = "/api/firmware",
        .method = HTTP_POST,
        .handler = firmware_post_handler,
        .user_ctx = NULL,
    };
    httpd_register_uri_handler(s_plc_http_server, &firmware_uri);
    ESP_LOGI(TAG, "OTA endpoint: POST /api/firmware");
#endif

    ESP_LOGI(TAG, "PLC Runtime ready - deploy bytecode via POST /api/deploy");
    ESP_LOGI(TAG, "WebSocket endpoint: ws://%s/ws/plc", "device-ip");

    return ESP_OK;
#else
    ESP_LOGE(TAG, "no protocol selected");
    return ESP_ERR_INVALID_STATE;
#endif
}
