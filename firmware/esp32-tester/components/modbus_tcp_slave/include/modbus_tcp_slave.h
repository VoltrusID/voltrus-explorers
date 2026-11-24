#pragma once

#include "esp_err.h"
#include "esp_netif.h"

esp_err_t modbus_tcp_slave_init(esp_netif_t *netif);
esp_err_t modbus_tcp_slave_start(void);
esp_err_t modbus_tcp_slave_stop(void);
