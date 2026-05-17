#pragma once

#include <stdint.h>
#include <stdbool.h>
#include "esp_err.h"

// Bytecode bundle magic number "VLT1"
#define PLC_BYTECODE_MAGIC  0x564C5431

// Bytecode format version
#define PLC_BYTECODE_VERSION  1

// Maximum limits for ESP32-S3 (constrained memory)
#define PLC_MAX_INSTRUCTIONS  4096
#define PLC_MAX_VARIABLES     512
#define PLC_MAX_IO_MAPPINGS   128
#define PLC_MAX_NAME_CONSTANTS 256
#define PLC_MAX_NAME_LEN      32

// Opcode definitions matching mt-plc-types exactly
typedef enum {
    // Push constants (0x01-0x07)
    OPC_PUSH_BOOL   = 0x01,
    OPC_PUSH_INT    = 0x02,
    OPC_PUSH_REAL   = 0x03,
    OPC_PUSH_TIME   = 0x04,
    OPC_PUSH_DINT   = 0x05,
    OPC_PUSH_ENUM   = 0x06,
    OPC_PUSH_STRING = 0x07,

    // Variable load/store (0x10-0x13)
    OPC_LOAD_VAR  = 0x10,
    OPC_STORE_VAR = 0x11,
    OPC_LOAD_IO   = 0x12,
    OPC_STORE_IO  = 0x13,

    // Arithmetic (0x20-0x26)
    OPC_ADD    = 0x20,
    OPC_SUB    = 0x21,
    OPC_MUL    = 0x22,
    OPC_DIV    = 0x23,
    OPC_MODULO = 0x24,
    OPC_POW    = 0x25,
    OPC_NEG    = 0x26,

    // Comparison (0x30-0x35)
    OPC_EQ = 0x30,
    OPC_NE = 0x31,
    OPC_LT = 0x32,
    OPC_LE = 0x33,
    OPC_GT = 0x34,
    OPC_GE = 0x35,

    // Logic / bitwise (0x40-0x47)
    OPC_AND = 0x40,
    OPC_OR  = 0x41,
    OPC_XOR = 0x42,
    OPC_NOT = 0x43,
    OPC_SHL = 0x44,
    OPC_SHR = 0x45,
    OPC_ROL = 0x46,
    OPC_ROR = 0x47,

    // Control flow (0x50-0x54)
    OPC_JUMP         = 0x50,
    OPC_JUMP_IF_FALSE = 0x51,
    OPC_CALL         = 0x52,
    OPC_CALL_POU     = 0x53,
    OPC_RETURN       = 0x54,

    // Function blocks (0x60-0x67)
    OPC_TON_CALL   = 0x60,
    OPC_TOF_CALL   = 0x61,
    OPC_TP_CALL    = 0x62,
    OPC_CTU_CALL   = 0x63,
    OPC_CTD_CALL   = 0x64,
    OPC_R_TRIG_CALL = 0x65,
    OPC_F_TRIG_CALL = 0x66,
    OPC_CTUD_CALL  = 0x67,

    // Math standard library (0x70-0x7E)
    OPC_SQRT = 0x70,
    OPC_LN   = 0x71,
    OPC_LOG  = 0x72,
    OPC_EXP  = 0x73,
    OPC_SIN  = 0x74,
    OPC_COS  = 0x75,
    OPC_TAN  = 0x76,
    OPC_ASIN = 0x77,
    OPC_ACOS = 0x78,
    OPC_ATAN = 0x79,
    OPC_ABS  = 0x7A,
    OPC_TRUNC = 0x7B,
    OPC_FLOOR = 0x7C,
    OPC_CEIL  = 0x7D,
    OPC_EXPT  = 0x7E,

    // String standard library (0x80-0x88)
    OPC_STR_LEN    = 0x80,
    OPC_STR_LEFT   = 0x81,
    OPC_STR_RIGHT  = 0x82,
    OPC_STR_MID    = 0x83,
    OPC_CONCAT     = 0x84,
    OPC_STR_INSERT = 0x85,
    OPC_STR_DELETE = 0x86,
    OPC_STR_REPLACE = 0x87,
    OPC_STR_FIND   = 0x88,

    // Conversion standard library (0x90-0x9D)
    OPC_INT_TO_REAL  = 0x90,
    OPC_REAL_TO_INT  = 0x91,
    OPC_INT_TO_BOOL  = 0x92,
    OPC_BOOL_TO_INT  = 0x93,
    OPC_INT_TO_STRING = 0x94,
    OPC_REAL_TO_STRING = 0x95,
    OPC_STRING_TO_INT  = 0x96,
    OPC_STRING_TO_REAL = 0x97,
    OPC_WORD_TO_INT    = 0x98,
    OPC_DWORD_TO_INT   = 0x99,
    OPC_INT_TO_WORD    = 0x9A,
    OPC_INT_TO_DWORD   = 0x9B,
    OPC_DWORD_TO_REAL  = 0x9C,
    OPC_REAL_TO_DWORD  = 0x9D,

    // Selection standard library (0xA0-0xA5)
    OPC_SEL   = 0xA0,
    OPC_MUX   = 0xA1,
    OPC_LIMIT = 0xA2,
    OPC_MAX   = 0xA3,
    OPC_MIN   = 0xA4,
    OPC_MOVE  = 0xA5,

    // Array ops (0xB0-0xB2)
    OPC_LOAD_ARRAY_INDEX  = 0xB0,
    OPC_STORE_ARRAY_INDEX = 0xB1,
    OPC_ARRAY_LEN         = 0xB2,

    // Struct ops (0xB3-0xB4)
    OPC_LOAD_STRUCT_FIELD  = 0xB3,
    OPC_STORE_STRUCT_FIELD = 0xB4,

    // Special
    OPC_HALT = 0xFF,
} plc_opcode_t;

// Variable type tag
typedef enum {
    PLC_TYPE_BOOL   = 0,
    PLC_TYPE_BYTE   = 1,
    PLC_TYPE_INT    = 2,
    PLC_TYPE_DINT   = 3,
    PLC_TYPE_REAL   = 4,
    PLC_TYPE_TIME   = 5,
    PLC_TYPE_STRING = 6,
    PLC_TYPE_WORD   = 7,
    PLC_TYPE_DWORD  = 8,
} plc_value_type_t;

// PLC value union (compact, fixed-size for embedded use)
typedef struct {
    plc_value_type_t type;
    union {
        bool    bool_val;
        uint8_t byte_val;
        int16_t int_val;
        int32_t dint_val;
        float   real_val;
        int32_t time_val;
        uint16_t word_val;
        uint32_t dword_val;
        // String/Enum stored as index into name_constants table
        uint16_t str_idx;
    };
} plc_value_t;

// Single instruction: opcode + operand (matches Rust Instruction layout)
typedef struct {
    uint8_t  opcode;
    int32_t  operand;
    uint8_t  has_operand;  // 0 = no operand, 1 = operand present
} plc_instruction_t;

// Bytecode bundle header (on-wire format)
typedef struct {
    uint32_t magic;              // PLC_BYTECODE_MAGIC
    uint16_t version;            // PLC_BYTECODE_VERSION
    uint32_t instruction_count;
    uint32_t variable_count;
    uint32_t io_mapping_count;
    uint32_t name_constant_count;
    // Followed by:
    //   instructions[instruction_count]  (each: 1 byte opcode + 1 byte has_operand + 4 byte operand)
    //   variable_init[variable_count]    (each: 1 byte type + 4 byte value)
    //   variable_names[variable_count]   (each: 1 byte name_len + name bytes)
    //   io_mappings[io_mapping_count]    (each: 4 byte variable_index + 1 byte io_type + 1 byte protocol + 4 byte address)
    //   name_constants[name_constant_count] (each: 1 byte name_len + name bytes)
    //   4 byte CRC32
} plc_bytecode_header_t;

// I/O mapping entry
typedef struct {
    uint32_t variable_index;
    uint8_t  io_type;    // 0=input, 1=output
    uint8_t  protocol;   // 0=none, 1=modbus, 2=s7, 3=enip, 4=gpio, 5=mqtt, 6=opcua
    uint32_t address;    // protocol-specific address
} plc_io_mapping_t;

// Bytecode loader context
typedef struct {
    plc_instruction_t *instructions;
    uint32_t instruction_count;
    plc_value_t *variable_init;
    char (*variable_names)[PLC_MAX_NAME_LEN];
    uint32_t variable_count;
    plc_io_mapping_t *io_mappings;
    uint32_t io_mapping_count;
    char (*name_constants)[PLC_MAX_NAME_LEN];
    uint32_t name_constant_count;
} plc_bytecode_t;

/**
 * Load and validate a bytecode bundle from raw bytes.
 * Allocates memory for instructions, variables, mappings.
 * Caller must free with plc_bytecode_free().
 */
esp_err_t plc_bytecode_load(const uint8_t *data, uint32_t len, plc_bytecode_t *out);

/**
 * Free all memory allocated by plc_bytecode_load.
 */
void plc_bytecode_free(plc_bytecode_t *bc);

/**
 * Validate bytecode integrity (magic, version, CRC).
 */
esp_err_t plc_bytecode_validate(const uint8_t *data, uint32_t len);
