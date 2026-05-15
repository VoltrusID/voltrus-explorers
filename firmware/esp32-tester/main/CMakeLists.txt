idf_component_register(
    SRCS "main.c" "wifi_station.c" "protocol_launcher.c"
    INCLUDE_DIRS "."
    REQUIRES esp_netif esp_wifi esp_event nvs_flash driver board
             modbus_tcp_slave modbus_tcp_master modbus_rtu_slave
             mqtt_client s7_server enip_server opcua_server
             bacnet_device dnp3_outstation serial_bridge
)
