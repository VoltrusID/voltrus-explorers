#include "my_mqtt_client.h"
#include "esp_log.h"
#include "esp_random.h"
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "mqtt_client.h"

static const char *TAG = "mqtt_client";

#define LED_PIN 2
#define PUBLISH_INTERVAL_MS CONFIG_MQTT_PUBLISH_INTERVAL_MS

static esp_mqtt_client_handle_t mqtt_client = NULL;
static bool mqtt_connected = false;
static uint32_t publish_counter = 0;

static void mqtt_event_handler(void *handler_args, esp_event_base_t base,
                                int32_t event_id, void *event_data)
{
    (void)handler_args;
    (void)base;

    esp_mqtt_event_handle_t event = event_data;

    switch (event_id) {
    case MQTT_EVENT_CONNECTED:
        ESP_LOGI(TAG, "MQTT connected to broker");
        mqtt_connected = true;
        esp_mqtt_client_subscribe_single(event->client, "esp32/control/led", 1);
        ESP_LOGI(TAG, "Subscribed to esp32/control/led");
        break;

    case MQTT_EVENT_DISCONNECTED:
        ESP_LOGW(TAG, "MQTT disconnected from broker");
        mqtt_connected = false;
        break;

    case MQTT_EVENT_DATA:
        ESP_LOGI(TAG, "MQTT message received: topic=%.*s data=%.*s",
                 event->topic_len, event->topic,
                 event->data_len, event->data);
        if (event->topic_len == 18 && strncmp(event->topic, "esp32/control/led", 18) == 0) {
            if (event->data_len >= 2 && strncmp(event->data, "ON", 2) == 0) {
                gpio_set_level(LED_PIN, 1);
                ESP_LOGI(TAG, "LED turned ON");
            } else if (event->data_len >= 3 && strncmp(event->data, "OFF", 3) == 0) {
                gpio_set_level(LED_PIN, 0);
                ESP_LOGI(TAG, "LED turned OFF");
            }
        }
        break;

    case MQTT_EVENT_SUBSCRIBED:
        ESP_LOGI(TAG, "MQTT subscribed, msg_id=%d", event->msg_id);
        break;

    case MQTT_EVENT_PUBLISHED:
        ESP_LOGD(TAG, "MQTT published, msg_id=%d", event->msg_id);
        break;

    case MQTT_EVENT_ERROR:
        ESP_LOGE(TAG, "MQTT error");
        break;

    default:
        break;
    }
}

static void publish_task(void *arg)
{
    (void)arg;

    while (1) {
        vTaskDelay(pdMS_TO_TICKS(PUBLISH_INTERVAL_MS));

        if (!mqtt_connected) {
            continue;
        }

        float temperature = 20.0f + (esp_random() % 1600) / 100.0f;
        float humidity = 40.0f + (esp_random() % 4000) / 100.0f;
        publish_counter++;

        char payload[64];

        snprintf(payload, sizeof(payload), "%.2f", temperature);
        int msg_id = esp_mqtt_client_publish(mqtt_client, "esp32/sensor/temperature",
                                              payload, 0, 1, 0);
        if (msg_id >= 0) {
            ESP_LOGI(TAG, "Published temperature: %s C (msg_id=%d)", payload, msg_id);
        }

        snprintf(payload, sizeof(payload), "%.2f", humidity);
        msg_id = esp_mqtt_client_publish(mqtt_client, "esp32/sensor/humidity",
                                          payload, 0, 1, 0);
        if (msg_id >= 0) {
            ESP_LOGI(TAG, "Published humidity: %s %% (msg_id=%d)", payload, msg_id);
        }

        snprintf(payload, sizeof(payload), "%lu", (unsigned long)publish_counter);
        msg_id = esp_mqtt_client_publish(mqtt_client, "esp32/sensor/counter",
                                          payload, 0, 1, 0);
        if (msg_id >= 0) {
            ESP_LOGI(TAG, "Published counter: %s (msg_id=%d)", payload, msg_id);
        }
    }
}

esp_err_t mqtt_client_init(void)
{
    gpio_set_direction(LED_PIN, GPIO_MODE_OUTPUT);
    gpio_set_level(LED_PIN, 0);

    ESP_LOGI(TAG, "MQTT Client initialized (broker=%s, interval=%dms)",
             CONFIG_MQTT_BROKER_URL, PUBLISH_INTERVAL_MS);
    return ESP_OK;
}

esp_err_t mqtt_client_start(void)
{
    esp_mqtt_client_config_t cfg = {
        .broker.address.uri = CONFIG_MQTT_BROKER_URL,
    };

    mqtt_client = esp_mqtt_client_init(&cfg);
    if (mqtt_client == NULL) {
        ESP_LOGE(TAG, "Failed to initialize MQTT client");
        return ESP_FAIL;
    }

    esp_mqtt_client_register_event(mqtt_client, ESP_EVENT_ANY_ID,
                                    mqtt_event_handler, NULL);
    esp_err_t ret = esp_mqtt_client_start(mqtt_client);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "Failed to start MQTT client: %s", esp_err_to_name(ret));
        return ret;
    }

    xTaskCreate(publish_task, "mqtt_pub", 4096, NULL,
                tskIDLE_PRIORITY + 3, NULL);

    ESP_LOGI(TAG, "MQTT Client started");
    return ESP_OK;
}
