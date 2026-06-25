import { linter, type Diagnostic } from "@codemirror/lint";
import { invoke } from "@tauri-apps/api/core";
import type { CompileResult } from "../../types";

export const stLinter = linter(
  async (view) => {
    const source = view.state.doc.toString();
    if (!source.trim()) return [];

    try {
      const result = await invoke<CompileResult>("compile_source", {
        source,
        fileName: "untitled.st",
      });

      return result.diagnostics
        .filter((d) => d.line > 0 || d.message.length > 0)
        .map((d): Diagnostic => {
          const line = Math.max(1, d.line);
          const col = Math.max(0, d.col);
          const from = Math.min(
            view.state.doc.line(line).from + col,
            view.state.doc.length
          );
          const toLine = d.end_line > 0 ? d.end_line : line;
          const toCol = d.end_col > 0 ? d.end_col : col + 1;
          const to = Math.min(
            view.state.doc.line(Math.min(toLine, view.state.doc.lines)).from + toCol,
            view.state.doc.length
          );
          return {
            from: Math.min(from, to),
            to: Math.max(from + 1, to),
            severity: d.severity === "error" ? "error" : d.severity === "warning" ? "warning" : "info",
            message: d.message,
            source: d.source,
          };
        });
    } catch {
      return [];
    }
  },
  { delay: 500 }
);
