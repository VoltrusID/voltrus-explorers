idf_component_register(
    SRCS "main.c" "wifi_station.c" "protocol_launcher.c"
    INCLUDE_DIRS "."
    REQUIRES esp_netif esp_wifi esp_event nvs_flash driver
)
