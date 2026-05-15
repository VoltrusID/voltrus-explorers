#include "board.h"
#include "esp_log.h"
#include "driver/gpio.h"
#include "driver/ledc.h"
#include "driver/rmt_tx.h"
#include <string.h>

static const char *TAG = "board";

static const int RELAY_PINS[BOARD_NUM_RELAYS] = BOARD_RELAY_PINS;
static bool relay_states[BOARD_NUM_RELAYS];
static bool buzzer_state;

// WS2812: 1 LED = 24 bits = 24 RMT symbols + 1 reset symbol
#define WS2812_NUM_BITS 24
#define WS2812_SYMBOLS  (WS2812_NUM_BITS + 1)

// Tick = 100ns at 10MHz
#define T0H  4
#define T0L  8
#define T1H  7
#define T1L  6

static rmt_channel_handle_t rmt_chan;
static rmt_encoder_handle_t rmt_copy_enc;
static rmt_symbol_word_t rgb_symbols[WS2812_SYMBOLS];

// LEDC PWM for passive buzzer
#define BUZZER_LEDC_TIMER   LEDC_TIMER_0
#define BUZZER_LEDC_CHANNEL LEDC_CHANNEL_0
#define BUZZER_FREQ_HZ      2700

static void build_rgb_symbols(uint8_t r, uint8_t g, uint8_t b)
{
    uint8_t grb[3] = { g, r, b };
    int sym = 0;
    for (int byte = 0; byte < 3; byte++) {
        for (int bit = 7; bit >= 0; bit--) {
            bool one = (grb[byte] >> bit) & 1;
            rgb_symbols[sym].level0 = 1;
            rgb_symbols[sym].duration0 = one ? T1H : T0H;
            rgb_symbols[sym].level1 = 0;
            rgb_symbols[sym].duration1 = one ? T1L : T0L;
            sym++;
        }
    }
    // Reset: low >50us
    rgb_symbols[sym].level0 = 0;
    rgb_symbols[sym].duration0 = 500;
    rgb_symbols[sym].level1 = 0;
    rgb_symbols[sym].duration1 = 0;
}

static void update_rgb(uint8_t r, uint8_t g, uint8_t b)
{
    build_rgb_symbols(r, g, b);
    rmt_transmit_config_t tx_cfg = { .loop_count = 0 };
    rmt_transmit(rmt_chan, rmt_copy_enc, rgb_symbols, sizeof(rgb_symbols), &tx_cfg);
    rmt_tx_wait_all_done(rmt_chan, 100);
}

void board_init(void)
{
    // Relays
    for (int i = 0; i < BOARD_NUM_RELAYS; i++) {
        gpio_set_direction(RELAY_PINS[i], GPIO_MODE_OUTPUT);
        gpio_set_level(RELAY_PINS[i], 0);
        relay_states[i] = false;
    }

    // Buzzer via LEDC PWM (passive buzzer needs AC tone, not DC)
    ledc_timer_config_t timer_cfg = {
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .duty_resolution = LEDC_TIMER_10_BIT,
        .timer_num = BUZZER_LEDC_TIMER,
        .freq_hz = BUZZER_FREQ_HZ,
        .clk_cfg = LEDC_AUTO_CLK,
    };
    ledc_timer_config(&timer_cfg);

    ledc_channel_config_t chan_cfg = {
        .gpio_num = BOARD_BUZZER_PIN,
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .channel = BUZZER_LEDC_CHANNEL,
        .timer_sel = BUZZER_LEDC_TIMER,
        .duty = 0,
        .hpoint = 0,
    };
    ledc_channel_config(&chan_cfg);
    buzzer_state = false;

    // WS2812 RGB via RMT
    rmt_tx_channel_config_t rmt_cfg = {
        .gpio_num = BOARD_RGB_PIN,
        .clk_src = RMT_CLK_SRC_DEFAULT,
        .resolution_hz = 10 * 1000 * 1000,
        .mem_block_symbols = 64,
        .trans_queue_depth = 4,
    };
    rmt_new_tx_channel(&rmt_cfg, &rmt_chan);
    rmt_copy_encoder_config_t copy_cfg = {};
    rmt_new_copy_encoder(&copy_cfg, &rmt_copy_enc);
    rmt_enable(rmt_chan);
    update_rgb(0, 0, 0);

    ESP_LOGI(TAG, "Board init: 6 relays, buzzer (LEDC %dHz), RGB WS2812", BUZZER_FREQ_HZ);
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
    // 50% duty cycle for max volume on passive buzzer
    ledc_set_duty(LEDC_LOW_SPEED_MODE, BUZZER_LEDC_CHANNEL, on ? 512 : 0);
    ledc_update_duty(LEDC_LOW_SPEED_MODE, BUZZER_LEDC_CHANNEL);
}

void board_set_rgb(uint8_t r, uint8_t g, uint8_t b)
{
    update_rgb(r, g, b);
}
