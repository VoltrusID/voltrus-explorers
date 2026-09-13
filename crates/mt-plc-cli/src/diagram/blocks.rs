//! Static mirror of the frontend block library
//! (`apps/voltrus-plc/src/types/fbd.ts` `BLOCK_LIBRARY`). Pin order and
//! `stTemplate` strings are load-bearing: the transpilers iterate pins in
//! declaration order and substitute `{{PIN}}` placeholders into the template.

pub struct PinDef {
    pub name: &'static str,
    pub data_type: &'static str,
    pub input: bool,
}

pub struct BlockDef {
    pub block_type: &'static str,
    pub pins: &'static [PinDef],
    pub st_template: &'static str,
    pub is_function_block: bool,
}

const fn pin(name: &'static str, data_type: &'static str, input: bool) -> PinDef {
    PinDef { name, data_type, input }
}

macro_rules! logic_block {
    ($t:literal, $op:literal) => {
        BlockDef {
            block_type: $t,
            pins: &[
                pin("IN1", "BOOL", true),
                pin("IN2", "BOOL", true),
                pin("OUT", "BOOL", false),
            ],
            st_template: concat!("{{OUT}} := {{IN1}} ", $op, " {{IN2}};"),
            is_function_block: false,
        }
    };
}

macro_rules! arith_block {
    ($t:literal, $op:literal) => {
        BlockDef {
            block_type: $t,
            pins: &[
                pin("IN1", "INT", true),
                pin("IN2", "INT", true),
                pin("OUT", "INT", false),
            ],
            st_template: concat!("{{OUT}} := {{IN1}} ", $op, " {{IN2}};"),
            is_function_block: false,
        }
    };
}

macro_rules! cmp_block {
    ($t:literal, $op:literal) => {
        BlockDef {
            block_type: $t,
            pins: &[
                pin("IN1", "INT", true),
                pin("IN2", "INT", true),
                pin("OUT", "BOOL", false),
            ],
            st_template: concat!("{{OUT}} := {{IN1}} ", $op, " {{IN2}};"),
            is_function_block: false,
        }
    };
}

macro_rules! timer_block {
    ($t:literal) => {
        BlockDef {
            block_type: $t,
            pins: &[
                pin("IN", "BOOL", true),
                pin("PT", "TIME", true),
                pin("Q", "BOOL", false),
                pin("ET", "TIME", false),
            ],
            st_template: "{{INSTANCE}}(IN := {{IN}}, PT := {{PT}});",
            is_function_block: true,
        }
    };
}

/// The block library, in the exact order of the TS `BLOCK_LIBRARY`.
pub static BLOCK_LIBRARY: &[BlockDef] = &[
    logic_block!("AND", "AND"),
    logic_block!("OR", "OR"),
    BlockDef {
        block_type: "NOT",
        pins: &[pin("IN", "BOOL", true), pin("OUT", "BOOL", false)],
        st_template: "{{OUT}} := NOT {{IN}};",
        is_function_block: false,
    },
    logic_block!("XOR", "XOR"),
    arith_block!("ADD", "+"),
    arith_block!("SUB", "-"),
    arith_block!("MUL", "*"),
    arith_block!("DIV", "/"),
    arith_block!("MOD", "MOD"),
    cmp_block!("GT", ">"),
    cmp_block!("LT", "<"),
    cmp_block!("GE", ">="),
    cmp_block!("LE", "<="),
    cmp_block!("EQ", "="),
    cmp_block!("NE", "<>"),
    timer_block!("TON"),
    timer_block!("TOF"),
    timer_block!("TP"),
    BlockDef {
        block_type: "CTU",
        pins: &[
            pin("CU", "BOOL", true),
            pin("R", "BOOL", true),
            pin("PV", "INT", true),
            pin("Q", "BOOL", false),
            pin("CV", "INT", false),
        ],
        st_template: "{{INSTANCE}}(CU := {{CU}}, R := {{R}}, PV := {{PV}});",
        is_function_block: true,
    },
    BlockDef {
        block_type: "CTD",
        pins: &[
            pin("CD", "BOOL", true),
            pin("LD", "BOOL", true),
            pin("PV", "INT", true),
            pin("Q", "BOOL", false),
            pin("CV", "INT", false),
        ],
        st_template: "{{INSTANCE}}(CD := {{CD}}, LD := {{LD}}, PV := {{PV}});",
        is_function_block: true,
    },
    BlockDef {
        block_type: "MOVE",
        pins: &[pin("IN", "INT", true), pin("OUT", "INT", false)],
        st_template: "{{OUT}} := {{IN}};",
        is_function_block: false,
    },
    BlockDef {
        block_type: "SR",
        pins: &[
            pin("SET", "BOOL", true),
            pin("RESET1", "BOOL", true),
            pin("Q1", "BOOL", false),
        ],
        st_template: "{{INSTANCE}}(SET := {{SET}}, RESET1 := {{RESET1}});",
        is_function_block: true,
    },
    BlockDef {
        block_type: "RS",
        pins: &[
            pin("SET1", "BOOL", true),
            pin("RESET", "BOOL", true),
            pin("Q1", "BOOL", false),
        ],
        st_template: "{{INSTANCE}}(SET1 := {{SET1}}, RESET := {{RESET}});",
        is_function_block: true,
    },
    BlockDef {
        block_type: "R_TRIG",
        pins: &[pin("CLK", "BOOL", true), pin("Q", "BOOL", false)],
        st_template: "{{INSTANCE}}(CLK := {{CLK}});",
        is_function_block: true,
    },
    BlockDef {
        block_type: "F_TRIG",
        pins: &[pin("CLK", "BOOL", true), pin("Q", "BOOL", false)],
        st_template: "{{INSTANCE}}(CLK := {{CLK}});",
        is_function_block: true,
    },
    BlockDef {
        block_type: "INPUT_PIN",
        pins: &[pin("OUT", "BOOL", false)],
        st_template: "",
        is_function_block: false,
    },
    BlockDef {
        block_type: "OUTPUT_PIN",
        pins: &[pin("IN", "BOOL", true)],
        st_template: "",
        is_function_block: false,
    },
];

/// `getBlockDef` — linear scan, first type match.
pub fn block_def(block_type: &str) -> Option<&'static BlockDef> {
    BLOCK_LIBRARY.iter().find(|b| b.block_type == block_type)
}

impl BlockDef {
    pub fn pin(&self, name: &str) -> Option<&'static PinDef> {
        self.pins.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_matches_ts_count() {
        // 7 logic+arith+cmp groups (AND OR NOT XOR, ADD SUB MUL DIV MOD,
        // GT LT GE LE EQ NE) = 15, 3 timers, 2 counters, MOVE, SR RS,
        // R_TRIG F_TRIG, INPUT_PIN OUTPUT_PIN.
        assert_eq!(BLOCK_LIBRARY.len(), 27);
    }

    #[test]
    fn timers_are_function_blocks() {
        assert!(block_def("TON").unwrap().is_function_block);
        assert!(!block_def("AND").unwrap().is_function_block);
        assert!(block_def("NOPE").is_none());
    }

    #[test]
    fn template_substitution_targets_exist() {
        for def in BLOCK_LIBRARY {
            if def.st_template.contains("{{INSTANCE}}") {
                continue;
            }
            for pin in def.pins {
                let ph = format!("{{{{{}}}}}", pin.name);
                if !def.st_template.is_empty() {
                    assert!(def.st_template.contains(&ph), "{} missing {ph}", def.block_type);
                }
            }
        }
    }
}
