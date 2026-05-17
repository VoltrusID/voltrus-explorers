idf_component_register(
    SRCS "plc_vm.c"
         "plc_runtime.c"
         "plc_bytecode.c"
         "plc_io_bridge.c"
         "plc_ws_server.c"
         "plc_flatbuf.c"
    INCLUDE_DIRS "include"
    REQUIRES esp_http_server esp_timer esp_event esp_netif nvs_flash driver json
)
