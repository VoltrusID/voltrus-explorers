import { LanguageSupport, LRLanguage } from "@codemirror/language";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { parser } from "./iecst.js";

export const stLanguage = LRLanguage.define({
  name: "iecst",
  parser,
  languageData: {
    commentTokens: { line: "//", block: { open: "(*", close: "*)" } },
    closeBrackets: { brackets: ["(", "[", "'"] },
  },
});

export function iecstLanguage() {
  return new LanguageSupport(stLanguage, [stHighlighting]);
}

export const stHighlighting = syntaxHighlighting(
  HighlightStyle.define([
    { tag: t.keyword, color: "#c678dd", fontWeight: "bold" },
    { tag: t.definitionKeyword, color: "#c678dd", fontWeight: "bold" },
    { tag: t.controlKeyword, color: "#c678dd", fontWeight: "bold" },
    { tag: t.operatorKeyword, color: "#56b6c2" },
    { tag: t.typeName, color: "#e5c07b" },
    { tag: t.variableName, color: "#61afef" },
    { tag: t.number, color: "#d19a66" },
    { tag: t.string, color: "#98c379" },
    { tag: t.atom, color: "#d19a66" },
    { tag: t.comment, color: "#5c6370", fontStyle: "italic" },
    { tag: t.operator, color: "#56b6c2" },
    { tag: t.punctuation, color: "#abb2bf" },
    { tag: t.separator, color: "#abb2bf" },
  ])
);
