#include "wifi_station.h"
#include "esp_wifi.h"
#include "esp_netif.h"
#include "esp_event.h"
#include "nvs_flash.h"
#include "nvs.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "esp_log.h"
#include "esp_http_server.h"
#include <string.h>

static const char *TAG = "wifi_station";

#define WIFI_CONNECTED_BIT BIT0
#define WIFI_FAIL_BIT      BIT1

static EventGroupHandle_t s_wifi_event_group;
static int s_retry_count = 0;
static const int MAX_RETRY = 10;

#ifdef CONFIG_ESP_TESTER_WIFI_PORTAL_ENABLE
static const char *portal_html =
    "<!DOCTYPE html>"
    "<html><head><title>Voltrus PLC WiFi Setup</title>"
    "<meta name='viewport' content='width=device-width,initial-scale=1'>"
    "<style>"
    "body{font-family:sans-serif;max-width:400px;margin:40px auto;padding:0 20px;color:#333}"
    "h1{font-size:1.4em}"
    "input{width:100%;padding:10px;margin:8px 0;box-sizing:border-box;border:1px solid #ccc;border-radius:4px}"
    "button{width:100%;padding:12px;background:#2563eb;color:#fff;border:none;border-radius:4px;font-size:1em;cursor:pointer}"
    "button:hover{background:#1d4ed8}"
    "</style></head>"
    "<body>"
    "<h1>Voltrus PLC - WiFi Setup</h1>"
    "<form method='POST' action='/wifi-setup'>"
    "<label>SSID</label>"
    "<input name='ssid' placeholder='Your WiFi name' required>"
    "<label>Password</label>"
    "<input name='password' type='password' placeholder='Your WiFi password'>"
    "<button type='submit'>Connect &amp; Restart</button>"
    "</form>"
    "</body></html>";

static esp_err_t portal_get_handler(httpd_req_t *req) {
    httpd_resp_set_type(req, "text/html");
    httpd_resp_sendstr(req, portal_html);
    return ESP_OK;
}

static esp_err_t wifi_setup_post_handler(httpd_req_t *req) {
    char buf[512];
    int ret = httpd_req_recv(req, buf, sizeof(buf) - 1);
    if (ret <= 0) {
        httpd_resp_send_err(req, HTTPD_500_INTERNAL_SERVER_ERROR, "Failed to receive");
        return ESP_FAIL;
    }
    buf[ret] = '\0';

    char ssid[64] = {0};
    char password[64] = {0};

    // Parse URL-encoded form data: ssid=...&password=...
    char *pair = strtok(buf, "&");
    while (pair) {
        if (strncmp(pair, "ssid=", 5) == 0) {
            // Simple URL decode
            char *src = pair + 5;
            int i = 0;
            while (*src && i < sizeof(ssid) - 1) {
                if (*src == '+') { ssid[i++] = ' '; src++; }
                else if (*src == '%' && src[1] && src[2]) {
                    char hex[3] = {src[1], src[2], 0};
                    ssid[i++] = (char)strtol(hex, NULL, 16);
                    src += 3;
                } else {
                    ssid[i++] = *src++;
                }
            }
            ssid[i] = '\0';
        } else if (strncmp(pair, "password=", 9) == 0) {
            char *src = pair + 9;
            int i = 0;
            while (*src && i < sizeof(password) - 1) {
                if (*src == '+') { password[i++] = ' '; src++; }
                else if (*src == '%' && src[1] && src[2]) {
                    char hex[3] = {src[1], src[2], 0};
                    password[i++] = (char)strtol(hex, NULL, 16);
                    src += 3;
                } else {
                    password[i++] = *src++;
                }
            }
            password[i] = '\0';
        }
        pair = strtok(NULL, "&");
    }

    ESP_LOGI(TAG, "WiFi portal received SSID: %s", ssid);

    // Store credentials to NVS
    nvs_handle_t nvs_handle;
    if (nvs_open("wifi_creds", NVS_READWRITE, &nvs_handle) == ESP_OK) {
        nvs_set_str(nvs_handle, "ssid", ssid);
        nvs_set_str(nvs_handle, "password", password);
        nvs_commit(nvs_handle);
        nvs_close(nvs_handle);
        ESP_LOGI(TAG, "WiFi credentials saved to NVS");
    } else {
        ESP_LOGE(TAG, "Failed to open NVS for writing");
    }

    httpd_resp_set_type(req, "text/html");
    httpd_resp_sendstr(req,
        "<html><body><h2>Saved! Restarting...</h2></body></html>");

    vTaskDelay(pdMS_TO_TICKS(1000));
    esp_restart();
    return ESP_OK;
}

static void start_captive_portal(void) {
    ESP_LOGI(TAG, "Starting captive portal AP: Voltrus-Setup");

    wifi_config_t ap_config = {
        .ap = {
            .ssid = "Voltrus-Setup",
            .ssid_len = 0,
            .channel = 1,
            .password = "",
            .max_connection = 4,
            .authmode = WIFI_AUTH_OPEN,
        },
    };

    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_APSTA));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_AP, &ap_config));
    ESP_ERROR_CHECK(esp_wifi_start());

    // Start HTTP server for portal
    httpd_config_t config = HTTPD_DEFAULT_CONFIG();
    config.server_port = 80;
    config.max_uri_handlers = 4;
    httpd_handle_t portal_server = NULL;
    if (httpd_start(&portal_server, &config) == ESP_OK) {
        httpd_uri_t portal_uri = {
            .uri = "/",
            .method = HTTP_GET,
            .handler = portal_get_handler,
            .user_ctx = NULL,
        };
        httpd_register_uri_handler(portal_server, &portal_uri);

        httpd_uri_t setup_uri = {
            .uri = "/wifi-setup",
            .method = HTTP_POST,
            .handler = wifi_setup_post_handler,
            .user_ctx = NULL,
        };
        httpd_register_uri_handler(portal_server, &setup_uri);

        ESP_LOGI(TAG, "Captive portal running at http://192.168.4.1/");
    } else {
        ESP_LOGE(TAG, "Failed to start captive portal HTTP server");
    }
}
#endif // CONFIG_ESP_TESTER_WIFI_PORTAL_ENABLE

static void event_handler(void *arg, esp_event_base_t event_base,
                          int32_t event_id, void *event_data) {
    if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_START) {
        esp_wifi_connect();
    } else if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_DISCONNECTED) {
        if (s_retry_count < MAX_RETRY) {
            esp_wifi_connect();
            s_retry_count++;
            ESP_LOGI(TAG, "retrying connection (%d/%d)", s_retry_count, MAX_RETRY);
        } else {
            xEventGroupSetBits(s_wifi_event_group, WIFI_FAIL_BIT);
        }
    } else if (event_base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
        ip_event_got_ip_t *event = (ip_event_got_ip_t *)event_data;
        ESP_LOGI(TAG, "got ip:" IPSTR, IP2STR(&event->ip_info.ip));
        s_retry_count = 0;
        xEventGroupSetBits(s_wifi_event_group, WIFI_CONNECTED_BIT);
    }
}

static bool read_wifi_credentials_from_nvs(char *ssid, size_t ssid_len,
                                            char *password, size_t password_len) {
    nvs_handle_t nvs_handle;
    if (nvs_open("wifi_creds", NVS_READONLY, &nvs_handle) != ESP_OK) {
        return false;
    }

    esp_err_t err = nvs_get_str(nvs_handle, "ssid", ssid, &ssid_len);
    if (err != ESP_OK) {
        nvs_close(nvs_handle);
        return false;
    }

    err = nvs_get_str(nvs_handle, "password", password, &password_len);
    nvs_close(nvs_handle);

    return (err == ESP_OK);
}

esp_err_t wifi_station_init(esp_netif_t **out_netif) {
    ESP_ERROR_CHECK(nvs_flash_init());
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    esp_netif_t *sta_netif = esp_netif_create_default_wifi_sta();

    wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&cfg));

    s_wifi_event_group = xEventGroupCreate();

    ESP_ERROR_CHECK(esp_event_handler_instance_register(WIFI_EVENT,
        ESP_EVENT_ANY_ID, &event_handler, NULL, NULL));
    ESP_ERROR_CHECK(esp_event_handler_instance_register(IP_EVENT,
        IP_EVENT_STA_GOT_IP, &event_handler, NULL, NULL));

    // Try NVS credentials first, fall back to menuconfig defaults
    char ssid[64] = {0};
    char password[64] = {0};
    bool from_nvs = read_wifi_credentials_from_nvs(ssid, sizeof(ssid),
                                                    password, sizeof(password));

    if (from_nvs) {
        ESP_LOGI(TAG, "Using WiFi credentials from NVS (SSID: %s)", ssid);
    } else {
        strncpy(ssid, CONFIG_ESP_TESTER_WIFI_SSID, sizeof(ssid) - 1);
        strncpy(password, CONFIG_ESP_TESTER_WIFI_PASSWORD, sizeof(password) - 1);
        ESP_LOGI(TAG, "Using WiFi credentials from menuconfig (SSID: %s)", ssid);
    }

    wifi_config_t wifi_config = {
        .sta = {
            .threshold.authmode = WIFI_AUTH_WPA2_PSK,
        },
    };
    strncpy((char *)wifi_config.sta.ssid, ssid, sizeof(wifi_config.sta.ssid) - 1);
    strncpy((char *)wifi_config.sta.password, password, sizeof(wifi_config.sta.password) - 1);

    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &wifi_config));
    ESP_ERROR_CHECK(esp_wifi_start());

    ESP_LOGI(TAG, "wifi_station_init finished");

    EventBits_t bits = xEventGroupWaitBits(s_wifi_event_group,
        WIFI_CONNECTED_BIT | WIFI_FAIL_BIT, pdFALSE, pdFALSE, portMAX_DELAY);

    if (bits & WIFI_CONNECTED_BIT) {
        ESP_LOGI(TAG, "connected to AP SSID:%s", ssid);
        if (out_netif) *out_netif = sta_netif;
        return ESP_OK;
    } else {
        ESP_LOGE(TAG, "failed to connect to SSID:%s", ssid);

#ifdef CONFIG_ESP_TESTER_WIFI_PORTAL_ENABLE
        ESP_LOGI(TAG, "Starting WiFi configuration portal...");
        start_captive_portal();
        if (out_netif) *out_netif = sta_netif;
        // Return ESP_OK so the app can continue (portal is running)
        return ESP_OK;
#else
        return ESP_FAIL;
#endif
    }
}
