import { ExternalTokenizer, InputStream, Stack } from "@lezer/lr";

function readBlockComment(input: InputStream, pos: number): number {
  let depth = 1;
  let cur = pos;
  for (;;) {
    const ch = input.peek(cur - input.pos);
    if (ch < 0) return cur;
    if (ch === 40 && input.peek(cur + 1 - input.pos) === 42) { // (*
      depth++;
      cur += 2;
    } else if (ch === 42 && input.peek(cur + 1 - input.pos) === 41) { // *)
      depth--;
      if (depth === 0) return cur + 2;
      cur += 2;
    } else {
      cur++;
    }
  }
}

export const specialTokens = new ExternalTokenizer(
  (input: InputStream, _stack: Stack) => {
    const c0 = input.next;
    const c1 = input.peek(1);

    // Line comment: //
    if (c0 === 47 && c1 === 47) {
      let end = input.pos + 2;
      while (input.peek(end - input.pos) !== 10 && input.peek(end - input.pos) >= 0) end++;
      input.acceptToken(1, end - input.pos);
      return;
    }

    // Block comment: (* *)
    if (c0 === 40 && c1 === 42) {
      const end = readBlockComment(input, input.pos);
      input.acceptToken(2, end - input.pos);
      return;
    }
  },
  { contextual: true, fallback: true }
);
