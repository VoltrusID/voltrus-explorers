#include "protocol_launcher.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

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

static const char *TAG = "protocol_launcher";

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
#else
    ESP_LOGE(TAG, "no protocol selected");
    return ESP_ERR_INVALID_STATE;
#endif
}
