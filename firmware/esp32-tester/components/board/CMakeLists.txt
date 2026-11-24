idf_component_register(
    SRCS "src/board.c"
    INCLUDE_DIRS "include"
    REQUIRES driver esp_driver_ledc
)
