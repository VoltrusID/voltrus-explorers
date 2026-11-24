import { CompletionContext, autocompletion } from "@codemirror/autocomplete";

const keywords = [
  "PROGRAM",
  "END_PROGRAM",
  "FUNCTION",
  "END_FUNCTION",
  "FUNCTION_BLOCK",
  "END_FUNCTION_BLOCK",
  "VAR",
  "VAR_INPUT",
  "VAR_OUTPUT",
  "VAR_IN_OUT",
  "VAR_TEMP",
  "VAR_GLOBAL",
  "END_VAR",
  "TYPE",
  "END_TYPE",
  "IF",
  "THEN",
  "ELSIF",
  "ELSE",
  "END_IF",
  "CASE",
  "OF",
  "END_CASE",
  "FOR",
  "TO",
  "BY",
  "DO",
  "END_FOR",
  "WHILE",
  "END_WHILE",
  "REPEAT",
  "UNTIL",
  "END_REPEAT",
  "EXIT",
  "RETURN",
  "CONTINUE",
  "AND",
  "OR",
  "XOR",
  "NOT",
  "MOD",
  "TRUE",
  "FALSE",
];

const types = [
  "BOOL",
  "INT",
  "DINT",
  "LINT",
  "REAL",
  "LREAL",
  "TIME",
  "STRING",
  "BYTE",
  "WORD",
  "DWORD",
];

const functions = [
  { name: "TON", detail: "Timer On Delay", info: "TON(IN := BOOL, PT := TIME) -> Q, ET" },
  { name: "TOF", detail: "Timer Off Delay", info: "TOF(IN := BOOL, PT := TIME) -> Q, ET" },
  { name: "TP", detail: "Pulse Timer", info: "TP(IN := BOOL, PT := TIME) -> Q, ET" },
  { name: "CTU", detail: "Count Up", info: "CTU(CU := BOOL, R := BOOL, PV := INT) -> Q, CV" },
  { name: "CTD", detail: "Count Down", info: "CTD(CD := BOOL, LD := BOOL, PV := INT) -> Q, CV" },
  { name: "R_TRIG", detail: "Rising Edge", info: "R_TRIG(CLK := BOOL) -> Q" },
  { name: "F_TRIG", detail: "Falling Edge", info: "F_TRIG(CLK := BOOL) -> Q" },
  { name: "ABS", detail: "Absolute value", info: "ABS(x) -> numeric" },
  { name: "SQRT", detail: "Square root", info: "SQRT(x) -> REAL" },
  { name: "LN", detail: "Natural logarithm", info: "LN(x) -> REAL" },
  { name: "LOG", detail: "Log base 10", info: "LOG(x) -> REAL" },
  { name: "EXP", detail: "Exponential e^x", info: "EXP(x) -> REAL" },
  { name: "SIN", detail: "Sine", info: "SIN(x) -> REAL (radians)" },
  { name: "COS", detail: "Cosine", info: "COS(x) -> REAL (radians)" },
  { name: "TAN", detail: "Tangent", info: "TAN(x) -> REAL (radians)" },
  { name: "ASIN", detail: "Arc sine", info: "ASIN(x) -> REAL" },
  { name: "ACOS", detail: "Arc cosine", info: "ACOS(x) -> REAL" },
  { name: "ATAN", detail: "Arc tangent", info: "ATAN(x) -> REAL" },
  { name: "TRUNC", detail: "Truncate to integer", info: "TRUNC(x) -> DINT" },
  { name: "FLOOR", detail: "Floor to integer", info: "FLOOR(x) -> DINT" },
  { name: "CEIL", detail: "Ceiling to integer", info: "CEIL(x) -> DINT" },
  { name: "EXPT", detail: "Power (x^y)", info: "EXPT(x, y) -> REAL" },
  { name: "LEN", detail: "String length", info: "LEN(s) -> DINT" },
  { name: "LEFT", detail: "Left substring", info: "LEFT(s, n) -> STRING" },
  { name: "RIGHT", detail: "Right substring", info: "RIGHT(s, n) -> STRING" },
  { name: "MID", detail: "Middle substring", info: "MID(s, pos, n) -> STRING" },
  { name: "CONCAT", detail: "String concatenation", info: "CONCAT(a, b) -> STRING" },
  { name: "INSERT", detail: "Insert string", info: "INSERT(s, ins, pos) -> STRING" },
  { name: "DELETE", detail: "Delete substring", info: "DELETE(s, pos, n) -> STRING" },
  { name: "REPLACE", detail: "Replace substring", info: "REPLACE(s, rep, pos, n) -> STRING" },
  { name: "FIND", detail: "Find substring", info: "FIND(s, needle) -> DINT (1-based)" },
  { name: "INT_TO_REAL", detail: "INT to REAL", info: "INT_TO_REAL(x) -> REAL" },
  { name: "REAL_TO_INT", detail: "REAL to INT", info: "REAL_TO_INT(x) -> DINT" },
  { name: "BOOL_TO_INT", detail: "BOOL to INT", info: "BOOL_TO_INT(b) -> DINT" },
  { name: "INT_TO_BOOL", detail: "INT to BOOL", info: "INT_TO_BOOL(x) -> BOOL" },
  { name: "MAX", detail: "Maximum", info: "MAX(a, b) -> numeric" },
  { name: "MIN", detail: "Minimum", info: "MIN(a, b) -> numeric" },
  { name: "LIMIT", detail: "Limit value", info: "LIMIT(min, val, max) -> numeric" },
  { name: "SEL", detail: "Select (binary)", info: "SEL(g, in0, in1) -> any" },
  { name: "MUX", detail: "Multiplexer", info: "MUX(k, in0, in1, ...) -> any" },
  { name: "MOVE", detail: "Move/assign", info: "MOVE(in) -> any" },
];

const snippets = [
  {
    name: "program",
    detail: "PROGRAM template",
    apply: "PROGRAM ${Name}\nVAR\n    \nEND_VAR\n\n    \nEND_PROGRAM",
  },
  {
    name: "function",
    detail: "FUNCTION template",
    apply: "FUNCTION ${Name} : ${INT}\nVAR_INPUT\n    \nEND_VAR\n\n    \nEND_FUNCTION",
  },
  {
    name: "function_block",
    detail: "FUNCTION_BLOCK template",
    apply: "FUNCTION_BLOCK ${Name}\nVAR_INPUT\n    \nEND_VAR\n\n    \nEND_FUNCTION_BLOCK",
  },
  {
    name: "if",
    detail: "IF statement",
    apply: "IF ${condition} THEN\n    \nEND_IF;",
  },
  {
    name: "if_else",
    detail: "IF-ELSE statement",
    apply: "IF ${condition} THEN\n    \nELSE\n    \nEND_IF;",
  },
  {
    name: "for",
    detail: "FOR loop",
    apply: "FOR ${i} := ${1} TO ${10} BY ${1} DO\n    \nEND_FOR;",
  },
  {
    name: "while",
    detail: "WHILE loop",
    apply: "WHILE ${condition} DO\n    \nEND_WHILE;",
  },
  {
    name: "case",
    detail: "CASE statement",
    apply: "CASE ${selector} OF\n    ${1}: \n    ELSE\n    \nEND_CASE;",
  },
  {
    name: "ton",
    detail: "TON timer instance",
    apply: "${timer} : TON;\n${timer}(IN := ${FALSE}, PT := T#${1}s);",
  },
];

export function stCompletions(context: CompletionContext) {
  const word = context.matchBefore(/[\w]*/);
  if (!word || (word.from === word.to && !context.explicit)) return null;

  const text = word.text.toUpperCase();

  const options = [
    ...keywords
      .filter((k) => k.startsWith(text))
      .map((k) => ({ label: k, type: "keyword", boost: 2 })),
    ...types
      .filter((t) => t.startsWith(text))
      .map((t) => ({ label: t, type: "type", boost: 1 })),
    ...functions
      .filter((f) => f.name.startsWith(text))
      .map((f) => ({ label: f.name, type: "function", detail: f.detail, info: f.info })),
    ...snippets
      .filter((s) => s.name.startsWith(text.toLowerCase()))
      .map((s) => ({ label: s.name, type: "text", detail: s.detail, apply: s.apply, boost: -1 })),
  ];

  return { from: word.from, options };
}

export const stCompletion = autocompletion({
  override: [stCompletions],
});
