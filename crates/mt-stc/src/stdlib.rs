use mt_plc_types::Opcode;

pub struct StdFuncInfo {
    pub name: &'static str,
    pub opcode: Opcode,
    pub arg_count: usize,
    pub description: &'static str,
}

pub fn standard_functions() -> &'static [StdFuncInfo] {
    &[
        // Math
        StdFuncInfo { name: "SQRT", opcode: Opcode::Sqrt, arg_count: 1, description: "Square root" },
        StdFuncInfo { name: "LN", opcode: Opcode::Ln, arg_count: 1, description: "Natural logarithm" },
        StdFuncInfo { name: "LOG", opcode: Opcode::Log, arg_count: 1, description: "Log base 10" },
        StdFuncInfo { name: "EXP", opcode: Opcode::Exp, arg_count: 1, description: "Exponential e^x" },
        StdFuncInfo { name: "SIN", opcode: Opcode::Sin, arg_count: 1, description: "Sine (radians)" },
        StdFuncInfo { name: "COS", opcode: Opcode::Cos, arg_count: 1, description: "Cosine (radians)" },
        StdFuncInfo { name: "TAN", opcode: Opcode::Tan, arg_count: 1, description: "Tangent (radians)" },
        StdFuncInfo { name: "ASIN", opcode: Opcode::Asin, arg_count: 1, description: "Arc sine" },
        StdFuncInfo { name: "ACOS", opcode: Opcode::Acos, arg_count: 1, description: "Arc cosine" },
        StdFuncInfo { name: "ATAN", opcode: Opcode::Atan, arg_count: 1, description: "Arc tangent" },
        StdFuncInfo { name: "ABS", opcode: Opcode::Abs, arg_count: 1, description: "Absolute value" },
        StdFuncInfo { name: "TRUNC", opcode: Opcode::Trunc, arg_count: 1, description: "Truncate to integer" },
        StdFuncInfo { name: "FLOOR", opcode: Opcode::Floor, arg_count: 1, description: "Floor to integer" },
        StdFuncInfo { name: "CEIL", opcode: Opcode::Ceil, arg_count: 1, description: "Ceiling to integer" },
        StdFuncInfo { name: "EXPT", opcode: Opcode::Expt, arg_count: 2, description: "Power x^y" },
        // Bitwise
        StdFuncInfo { name: "SHL", opcode: Opcode::Shl, arg_count: 2, description: "Shift left" },
        StdFuncInfo { name: "SHR", opcode: Opcode::Shr, arg_count: 2, description: "Shift right" },
        StdFuncInfo { name: "ROL", opcode: Opcode::Rol, arg_count: 2, description: "Rotate left" },
        StdFuncInfo { name: "ROR", opcode: Opcode::Ror, arg_count: 2, description: "Rotate right" },
        // String
        StdFuncInfo { name: "LEN", opcode: Opcode::StrLen, arg_count: 1, description: "String length" },
        StdFuncInfo { name: "LEFT", opcode: Opcode::StrLeft, arg_count: 2, description: "Left substring" },
        StdFuncInfo { name: "RIGHT", opcode: Opcode::StrRight, arg_count: 2, description: "Right substring" },
        StdFuncInfo { name: "MID", opcode: Opcode::StrMid, arg_count: 3, description: "Middle substring" },
        StdFuncInfo { name: "CONCAT", opcode: Opcode::Concat, arg_count: 2, description: "Concatenate strings" },
        StdFuncInfo { name: "INSERT", opcode: Opcode::StrInsert, arg_count: 3, description: "Insert string" },
        StdFuncInfo { name: "DELETE", opcode: Opcode::StrDelete, arg_count: 3, description: "Delete substring" },
        StdFuncInfo { name: "REPLACE", opcode: Opcode::StrReplace, arg_count: 4, description: "Replace substring" },
        StdFuncInfo { name: "FIND", opcode: Opcode::StrFind, arg_count: 2, description: "Find substring" },
        // Conversion
        StdFuncInfo { name: "INT_TO_REAL", opcode: Opcode::IntToReal, arg_count: 1, description: "INT to REAL" },
        StdFuncInfo { name: "REAL_TO_INT", opcode: Opcode::RealToInt, arg_count: 1, description: "REAL to INT" },
        StdFuncInfo { name: "INT_TO_BOOL", opcode: Opcode::IntToBool, arg_count: 1, description: "INT to BOOL" },
        StdFuncInfo { name: "BOOL_TO_INT", opcode: Opcode::BoolToInt, arg_count: 1, description: "BOOL to INT" },
        StdFuncInfo { name: "INT_TO_STRING", opcode: Opcode::IntToString, arg_count: 1, description: "INT to STRING" },
        StdFuncInfo { name: "REAL_TO_STRING", opcode: Opcode::RealToString, arg_count: 1, description: "REAL to STRING" },
        StdFuncInfo { name: "STRING_TO_INT", opcode: Opcode::StringToInt, arg_count: 1, description: "STRING to INT" },
        StdFuncInfo { name: "STRING_TO_REAL", opcode: Opcode::StringToReal, arg_count: 1, description: "STRING to REAL" },
        // More conversions
        StdFuncInfo { name: "WORD_TO_INT", opcode: Opcode::WordToInt, arg_count: 1, description: "WORD to INT" },
        StdFuncInfo { name: "DWORD_TO_INT", opcode: Opcode::DwordToInt, arg_count: 1, description: "DWORD to INT" },
        StdFuncInfo { name: "INT_TO_WORD", opcode: Opcode::IntToWord, arg_count: 1, description: "INT to WORD" },
        StdFuncInfo { name: "INT_TO_DWORD", opcode: Opcode::IntToDword, arg_count: 1, description: "INT to DWORD" },
        StdFuncInfo { name: "DWORD_TO_REAL", opcode: Opcode::DwordToReal, arg_count: 1, description: "DWORD bits to REAL" },
        StdFuncInfo { name: "REAL_TO_DWORD", opcode: Opcode::RealToDword, arg_count: 1, description: "REAL bits to DWORD" },
        // Selection
        StdFuncInfo { name: "SEL", opcode: Opcode::Sel, arg_count: 3, description: "Binary select" },
        StdFuncInfo { name: "MUX", opcode: Opcode::Mux, arg_count: 0, description: "Multiplexer (varargs)" },
        StdFuncInfo { name: "LIMIT", opcode: Opcode::Limit, arg_count: 3, description: "Limit value" },
        StdFuncInfo { name: "MAX", opcode: Opcode::Max, arg_count: 2, description: "Maximum" },
        StdFuncInfo { name: "MIN", opcode: Opcode::Min, arg_count: 2, description: "Minimum" },
        StdFuncInfo { name: "MOVE", opcode: Opcode::Move, arg_count: 1, description: "Move/assign" },
    ]
}
