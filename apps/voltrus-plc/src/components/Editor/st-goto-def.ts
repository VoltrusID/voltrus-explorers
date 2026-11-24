import { EditorView } from "@codemirror/view";

const ST_KEYWORDS = new Set([
  "PROGRAM", "END_PROGRAM", "FUNCTION", "END_FUNCTION", "FUNCTION_BLOCK", "END_FUNCTION_BLOCK",
  "VAR", "VAR_INPUT", "VAR_OUTPUT", "VAR_IN_OUT", "END_VAR",
  "IF", "THEN", "ELSIF", "ELSE", "END_IF",
  "CASE", "OF", "END_CASE",
  "FOR", "TO", "BY", "DO", "END_FOR",
  "WHILE", "REPEAT", "UNTIL", "END_WHILE", "END_REPEAT",
  "RETURN", "EXIT", "CONTINUE", "NOT", "AND", "OR", "XOR", "MOD",
  "TRUE", "FALSE", "TON", "TOF", "TP", "CTU", "CTD", "CTUD", "R_TRIG", "F_TRIG",
  "INT", "DINT", "REAL", "BOOL", "BYTE", "WORD", "DWORD", "TIME", "STRING",
]);

function wordAtPos(doc: string, pos: number): { word: string; from: number; to: number } | null {
  if (pos < 0 || pos > doc.length) return null;

  let from = pos;
  while (from > 0 && /[a-zA-Z0-9_]/.test(doc[from - 1])) from--;
  let to = pos;
  while (to < doc.length && /[a-zA-Z0-9_]/.test(doc[to])) to++;

  const word = doc.slice(from, to);
  if (word.length === 0) return null;
  if (ST_KEYWORDS.has(word.toUpperCase())) return null;
  return { word, from, to };
}

function gotoDeclaration(view: EditorView, word: string) {
  const doc = view.state.doc.toString();
  const escaped = word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const declPattern = new RegExp(`\\b${escaped}\\s*\\s*:`, "i");
  const match = declPattern.exec(doc);
  if (!match) return;

  const line = view.state.doc.lineAt(match.index);
  view.dispatch({
    selection: { anchor: line.from },
    scrollIntoView: true,
  });
}

export const gotoDefExtension = EditorView.domEventHandlers({
  mousedown(event, view) {
    if (!event.metaKey && !event.ctrlKey) return false;
    if (event.button !== 0) return false;

    const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
    if (pos === null) return false;

    const docStr = view.state.doc.toString();
    const result = wordAtPos(docStr, pos);
    if (!result) return false;

    event.preventDefault();
    gotoDeclaration(view, result.word);
    return true;
  },
});
