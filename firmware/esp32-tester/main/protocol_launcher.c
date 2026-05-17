#include "protocol_launcher.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_http_server.h"
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

    ESP_LOGI(TAG, "PLC Runtime ready - deploy bytecode via POST /api/deploy");
    ESP_LOGI(TAG, "WebSocket endpoint: ws://%s/ws/plc", "device-ip");

    return ESP_OK;
#else
    ESP_LOGE(TAG, "no protocol selected");
    return ESP_ERR_INVALID_STATE;
#endif
}
