#pragma once

#include "esp_err.h"
#include "esp_netif.h"

esp_err_t protocol_launcher_start(esp_netif_t *netif);
