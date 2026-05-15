#include "board.h"
#include "esp_log.h"
#include "driver/gpio.h"
#include "driver/rmt_tx.h"
#include "esp_rom_sys.h"
#include <string.h>

static const char *TAG = "board";

static const int RELAY_PINS[BOARD_NUM_RELAYS] = BOARD_RELAY_PINS;
static bool relay_states[BOARD_NUM_RELAYS];
static bool buzzer_state;
static uint8_t rgb_r, rgb_g, rgb_b;

// WS2812 timing (nanoseconds) for RMT
#define WS2812_T0H_NS   320
#define WS2812_T0L_NS   800
#define WS2812_T1H_NS   640
#define WS2812_T1L_NS   480
#define WS2812_RESET_MS 1

static rmt_channel_handle_t rmt_chan;
static rmt_encoder_handle_t rmt_encoder;

// Custom RMT encoder for WS2812
typedef struct {
    rmt_encoder_t base;
    rmt_encoder_t *bytes_encoder;
    rmt_encoder_t *copy_encoder;
    int state;
} ws2812_encoder_t;

static size_t ws2812_encode(rmt_encoder_t *enc, rmt_channel_handle_t chan,
                            const void *data, size_t data_size,
                            rmt_encode_state_t *ret_state)
{
    ws2812_encoder_t *ws = __containerof(enc, ws2812_encoder_t, base);
    rmt_encode_state_t session_state = 0;
    size_t encoded = 0;

    if (ws->state == 0) {
        encoded = ws->bytes_encoder->encode(ws->bytes_encoder, chan, data, data_size, &session_state);
        if (session_state & RMT_ENCODING_COMPLETE) {
            ws->state = 1;
        }
        if (session_state & RMT_ENCODING_MEM_FULL) {
            *ret_state |= RMT_ENCODING_MEM_FULL;
            goto out;
        }
    }

    if (ws->state == 1) {
        // Reset code: low for >50us (use 1ms)
        rmt_symbol_word_t reset_sym = {
            .duration0 = 1000,
            .level0 = 0,
            .duration1 = 0,
            .level1 = 0,
        };
        encoded += ws->copy_encoder->encode(ws->copy_encoder, chan, &reset_sym, sizeof(reset_sym), &session_state);
        if (session_state & RMT_ENCODING_COMPLETE) {
            ws->state = 0;
            *ret_state |= RMT_ENCODING_COMPLETE;
        }
        if (session_state & RMT_ENCODING_MEM_FULL) {
            *ret_state |= RMT_ENCODING_MEM_FULL;
        }
    }

out:
    return encoded;
}

static esp_err_t ws2812_del(rmt_encoder_t *enc)
{
    ws2812_encoder_t *ws = __containerof(enc, ws2812_encoder_t, base);
    rmt_del_encoder(ws->bytes_encoder);
    rmt_del_encoder(ws->copy_encoder);
    free(ws);
    return ESP_OK;
}

static esp_err_t ws2812_reset(rmt_encoder_t *enc)
{
    ws2812_encoder_t *ws = __containerof(enc, ws2812_encoder_t, base);
    rmt_encoder_reset(ws->bytes_encoder);
    rmt_encoder_reset(ws->copy_encoder);
    ws->state = 0;
    return ESP_OK;
}

static esp_err_t create_ws2812_encoder(rmt_encoder_handle_t *ret)
{
    ws2812_encoder_t *ws = calloc(1, sizeof(ws2812_encoder_t));
    if (!ws) return ESP_ERR_NO_MEM;

    ws->base.encode = ws2812_encode;
    ws->base.del = ws2812_del;
    ws->base.reset = ws2812_reset;

    rmt_bytes_encoder_config_t bytes_cfg = {
        .bit0 = {
            .duration0 = WS2812_T0H_NS / 25,
            .level0 = 1,
            .duration1 = WS2812_T0L_NS / 25,
            .level1 = 0,
        },
        .bit1 = {
            .duration0 = WS2812_T1H_NS / 25,
            .level0 = 1,
            .duration1 = WS2812_T1L_NS / 25,
            .level1 = 0,
        },
        .flags.msb_first = 1,
    };
    rmt_new_bytes_encoder(&bytes_cfg, &ws->bytes_encoder);

    rmt_copy_encoder_config_t copy_cfg = {};
    rmt_new_copy_encoder(&copy_cfg, &ws->copy_encoder);

    ws->state = 0;
    *ret = &ws->base;
    return ESP_OK;
}

static void update_rgb(void)
{
    uint8_t grb[3] = { rgb_g, rgb_r, rgb_b };
    rmt_transmit_config_t tx_cfg = { .loop_count = 0 };
    rmt_transmit(rmt_chan, rmt_encoder, grb, 3, &tx_cfg);
    rmt_tx_wait_all_done(rmt_chan, -1);
}

void board_init(void)
{
    // Init relay GPIOs
    for (int i = 0; i < BOARD_NUM_RELAYS; i++) {
        gpio_set_direction(RELAY_PINS[i], GPIO_MODE_OUTPUT);
        gpio_set_level(RELAY_PINS[i], 0);
        relay_states[i] = false;
    }

    // Init buzzer
    gpio_set_direction(BOARD_BUZZER_PIN, GPIO_MODE_OUTPUT);
    gpio_set_level(BOARD_BUZZER_PIN, 0);
    buzzer_state = false;

    // Init WS2812 RGB via RMT
    rmt_tx_channel_config_t rmt_cfg = {
        .gpio_num = BOARD_RGB_PIN,
        .clk_src = RMT_CLK_SRC_DEFAULT,
        .resolution_hz = 10 * 1000 * 1000, // 10MHz = 100ns ticks
        .mem_block_symbols = 64,
        .trans_queue_depth = 4,
    };
    rmt_new_tx_channel(&rmt_cfg, &rmt_chan);
    create_ws2812_encoder(&rmt_encoder);
    rmt_enable(rmt_chan);

    rgb_r = 0; rgb_g = 0; rgb_b = 0;
    update_rgb();

    ESP_LOGI(TAG, "Board init: 6 relays (GPIO1-6), buzzer (GPIO21), RGB WS2812 (GPIO38)");
}

void board_set_relay(int ch, bool on)
{
    if (ch < 0 || ch >= BOARD_NUM_RELAYS) return;
    relay_states[ch] = on;
    gpio_set_level(RELAY_PINS[ch], on ? 1 : 0);
}

bool board_get_relay(int ch)
{
    if (ch < 0 || ch >= BOARD_NUM_RELAYS) return false;
    return relay_states[ch];
}

void board_set_buzzer(bool on)
{
    buzzer_state = on;
    gpio_set_level(BOARD_BUZZER_PIN, on ? 1 : 0);
}

void board_set_rgb(uint8_t r, uint8_t g, uint8_t b)
{
    rgb_r = r;
    rgb_g = g;
    rgb_b = b;
    update_rgb();
}
