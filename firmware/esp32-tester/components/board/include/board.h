#pragma once
#include <stdbool.h>
#include <stdint.h>

#define BOARD_NUM_RELAYS 6
#define BOARD_RELAY_PINS { 1, 2, 41, 42, 45, 46 }
#define BOARD_BUZZER_PIN 21
#define BOARD_RGB_PIN    38

void board_init(void);
void board_set_relay(int ch, bool on);
bool board_get_relay(int ch);
void board_set_buzzer(bool on);
void board_set_rgb(uint8_t r, uint8_t g, uint8_t b);
