import { Decoration, DecorationSet, EditorView, ViewPlugin, ViewUpdate, WidgetType } from "@codemirror/view";
import { RangeSetBuilder } from "@codemirror/state";

// ST token types and their regex patterns
const keywords = [
  "PROGRAM", "END_PROGRAM", "FUNCTION", "END_FUNCTION", "FUNCTION_BLOCK", "END_FUNCTION_BLOCK",
  "TYPE", "END_TYPE", "STRUCT", "END_STRUCT",
  "VAR_INPUT", "VAR_OUTPUT", "VAR_IN_OUT", "VAR_TEMP", "VAR_GLOBAL", "VAR_EXTERNAL", "VAR",
  "END_VAR",
  "IF", "THEN", "ELSIF", "ELSE", "END_IF",
  "CASE", "END_CASE",
  "FOR", "TO", "BY", "DO", "END_FOR",
  "WHILE", "END_WHILE",
  "REPEAT", "UNTIL", "END_REPEAT",
  "EXIT", "RETURN", "CONTINUE",
  "AND", "OR", "XOR", "NOT", "MOD",
];

const typeKeywords = ["ARRAY", "OF", "STRING", "BOOL", "INT", "DINT", "REAL", "LREAL", "BYTE", "WORD", "DWORD", "TIME", "DATE", "TOD", "DT"];

const atomKeywords = ["TRUE", "FALSE"];

const colorMap: Record<string, string> = {
  keyword: "st-kw",
  type: "st-type",
  atom: "st-atom",
  number: "st-num",
  string: "st-str",
  comment: "st-cmt",
  ident: "st-id",
  directAddr: "st-addr",
  operator: "st-op",
  punctuation: "st-punct",
};

const stTheme = EditorView.baseTheme({
  ".st-kw": { color: "#c678dd", fontWeight: "bold" },
  ".st-type": { color: "#e5c07b" },
  ".st-atom": { color: "#d19a66" },
  ".st-num": { color: "#d19a66" },
  ".st-str": { color: "#98c379" },
  ".st-cmt": { color: "#5c6370", fontStyle: "italic" },
  ".st-id": { color: "#61afef" },
  ".st-addr": { color: "#61afef" },
  ".st-op": { color: "#56b6c2" },
  ".st-punct": { color: "#abb2bf" },
});

function buildDecorations(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const doc = view.state.doc;

  for (let i = 1; i <= doc.lines; i++) {
    const line = doc.line(i);
    const text = line.text;
    const base = line.from;

    // Track comment state (simple per-line)
    let pos = 0;
    while (pos < text.length) {
      // Block comment (* ... *)
      if (text[pos] === "(" && pos + 1 < text.length && text[pos + 1] === "*") {
        const end = text.indexOf("*)", pos + 2);
        const commentEnd = end === -1 ? text.length : end + 2;
        builder.add(
          base + pos,
          base + commentEnd,
          Decoration.mark({ class: "st-cmt" })
        );
        pos = commentEnd;
        continue;
      }

      // Line comment //
      if (text[pos] === "/" && pos + 1 < text.length && text[pos + 1] === "/") {
        builder.add(base + pos, base + text.length, Decoration.mark({ class: "st-cmt" }));
        pos = text.length;
        continue;
      }

      // String literal 'xxx'
      if (text[pos] === "'") {
        let end = pos + 1;
        while (end < text.length && text[end] !== "'") end++;
        if (end < text.length) end++; // include closing quote
        builder.add(base + pos, base + end, Decoration.mark({ class: "st-str" }));
        pos = end;
        continue;
      }

      // Direct address %IX0.0 etc.
      if (text[pos] === "%") {
        const m = text.slice(pos).match(/^%[IQMiqm][XWDBxwdb]?\d+(\.\d+)*/);
        if (m) {
          builder.add(base + pos, base + pos + m[0].length, Decoration.mark({ class: "st-addr" }));
          pos += m[0].length;
          continue;
        }
      }

      // Number (hex, float, int)
      const numMatch = text.slice(pos).match(/^(?:0[xX][0-9a-fA-F]+|[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/);
      if (numMatch && (pos === 0 || !/[\w]/.test(text[pos - 1]))) {
        const after = pos + numMatch[0].length;
        if (after >= text.length || !/[\w]/.test(text[after])) {
          builder.add(base + pos, base + after, Decoration.mark({ class: "st-num" }));
          pos = after;
          continue;
        }
      }

      // Identifier / keyword
      const idMatch = text.slice(pos).match(/^[a-zA-Z_]\w*/);
      if (idMatch && (pos === 0 || !/[\w]/.test(text[pos - 1]))) {
        const word = idMatch[0].toUpperCase();
        const end = pos + idMatch[0].length;
        let cls: string | null = null;

        if (keywords.includes(word)) {
          cls = "st-kw";
        } else if (typeKeywords.includes(word)) {
          cls = "st-type";
        } else if (atomKeywords.includes(word)) {
          cls = "st-atom";
        } else if (pos > 0 && text[pos - 1] === ".") {
          // field access after dot — don't color
          pos = end;
          continue;
        } else if (end < text.length && text[end] === "(") {
          // function call — color as identifier
          cls = "st-id";
        } else {
          // plain identifier — check if it's followed by ":" (could be a type name)
          const afterWord = text.slice(end).trimStart();
          if (afterWord.startsWith(":") && !afterWord.startsWith(":=")) {
            // This is a declaration: counter : INT — don't color the name
            pos = end;
            continue;
          }
          cls = "st-id";
        }

        if (cls) {
          builder.add(base + pos, base + end, Decoration.mark({ class: cls }));
        }
        pos = end;
        continue;
      }

      // Operators
      if (":=<>+-*/=><".includes(text[pos])) {
        const opMatch = text.slice(pos).match(/^(?::=|<>|<=|>=|\*\*|[+\-*/=<>])/);
        if (opMatch) {
          builder.add(base + pos, base + pos + opMatch[0].length, Decoration.mark({ class: "st-op" }));
          pos += opMatch[0].length;
          continue;
        }
      }

      // Punctuation: parentheses, brackets, semicolons, commas, dots
      if ("()[];,.:..".includes(text[pos])) {
        builder.add(base + pos, base + pos + 1, Decoration.mark({ class: "st-punct" }));
        pos++;
        continue;
      }

      pos++;
    }
  }

  return builder.finish();
}

export const stColorPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  { decorations: (v) => v.decorations }
);

export { stTheme };
