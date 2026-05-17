#pragma once

#include <stdint.h>
#include "esp_err.h"
#include "esp_http_server.h"
#include "plc_vm.h"

// WebSocket protocol message types
#define PLC_WS_MSG_VAR_UPDATE  0x01
#define PLC_WS_MSG_FORCE       0x02
#define PLC_WS_MSG_UNFORCE     0x03
#define PLC_WS_MSG_START       0x04
#define PLC_WS_MSG_STOP        0x05
#define PLC_WS_MSG_STEP        0x06
#define PLC_WS_MSG_STATUS      0x07
#define PLC_WS_MSG_ERROR       0x08

/**
 * Initialize the WebSocket server and HTTP deploy endpoint.
 * Registers URI handlers on the provided HTTP server.
 * @param server  Already-started esp_http_server handle
 */
esp_err_t plc_ws_server_init(httpd_handle_t server);

/**
 * Start periodic variable push to connected WebSocket clients.
 * Called by the scan task after each cycle.
 */
esp_err_t plc_ws_push_variables(plc_vm_t *vm);

/**
 * Stop the WebSocket server and clean up.
 */
void plc_ws_server_stop(void);

/**
 * Check if any WebSocket client is connected.
 */
bool plc_ws_has_client(void);

/**
 * Get the HTTP server handle for deploy endpoint registration.
 * This is set externally when the main HTTP server is created.
 */
void plc_ws_set_http_server(httpd_handle_t server);
