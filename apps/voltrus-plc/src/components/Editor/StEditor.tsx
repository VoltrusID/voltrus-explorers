import { useEffect, useRef } from "react";
import { EditorState, StateField, StateEffect, RangeSet } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLineGutter, highlightActiveLine, drawSelection, rectangularSelection, crosshairCursor, highlightSpecialChars, gutter, GutterMarker } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { syntaxHighlighting, indentOnInput, bracketMatching, foldGutter, foldKeymap, defaultHighlightStyle } from "@codemirror/language";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { lintKeymap, lintGutter } from "@codemirror/lint";
import { stLanguageSupport, stHighlighting } from "./st-language";
import { stColorPlugin, stTheme } from "./st-colors";
import { stCompletion } from "./st-completion";
import { stLinter } from "./st-lint";
import { gotoDefExtension } from "./st-goto-def";
import { useStore } from "../../store";

interface StEditorProps {
  value: string;
  onChange: (value: string) => void;
}

const bpMarker = new (class extends GutterMarker {
  toDOM() {
    const dot = document.createElement("span");
    dot.className = "breakpoint-marker";
    return dot;
  }
})();

const exeMarker = new (class extends GutterMarker {
  toDOM() {
    const arrow = document.createElement("span");
    arrow.className = "execution-line-marker";
    return arrow;
  }
})();

const setBreakpoints = StateEffect.define<number[]>();
const setExecutionLine = StateEffect.define<number | null>();

const breakpointField = StateField.define<RangeSet<GutterMarker>>({
  create() {
    return RangeSet.empty;
  },
  update(marks, tr) {
    let bpLines: number[] | undefined;
    let exeLine: number | null | undefined;
    for (const e of tr.effects) {
      if (e.is(setBreakpoints)) bpLines = e.value;
      if (e.is(setExecutionLine)) exeLine = e.value;
    }
    if (bpLines === undefined && exeLine === undefined) return marks;

    const currentBps = bpLines ?? useStore.getState().breakpoints;
    const currentExe = exeLine !== undefined ? exeLine : useStore.getState().currentExecutionLine;

    const doc = tr.state.doc;
    const iter = marks.iter();
    let result = RangeSet.empty<GutterMarker>();

    for (const line of currentBps) {
      if (line >= 1 && line <= doc.lines) {
        const pos = doc.line(line).from;
        result = result.update({ add: [{ from: pos, to: pos, value: bpMarker }] });
      }
    }
    if (currentExe !== null && currentExe >= 1 && currentExe <= doc.lines) {
      const pos = doc.line(currentExe).from;
      result = result.update({ add: [{ from: pos, to: pos, value: exeMarker }] });
    }
    return result;
  },
});

const breakpointGutter = gutter({
  class: "breakpoint-gutter",
  markers: (view) => view.state.field(breakpointField),
  initialSpacer: () => bpMarker,
  domEventHandlers: {
    click(view, line) {
      const store = useStore.getState();
      const lineNo = view.state.doc.lineAt(line.from).number;
      if (store.breakpoints.includes(lineNo)) {
        store.removeBreakpoint(lineNo);
      } else {
        store.addBreakpoint(lineNo);
      }
      view.dispatch({
        effects: setBreakpoints.of(store.breakpoints),
      });
      return true;
    },
  },
});

export default function StEditor({ value, onChange }: StEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    if (!containerRef.current) return;

    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightSpecialChars(),
        history(),
        foldGutter(),
        drawSelection(),
        EditorState.allowMultipleSelections.of(true),
        indentOnInput(),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        bracketMatching(),
        closeBrackets(),
        rectangularSelection(),
        crosshairCursor(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        keymap.of([
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...searchKeymap,
          ...historyKeymap,
          ...foldKeymap,
          ...lintKeymap,
          indentWithTab,
        ]),
        stLanguageSupport,
        stHighlighting,
        stTheme,
        stColorPlugin,
        stCompletion,
        stLinter,
        gotoDefExtension,
        lintGutter(),
        breakpointField,
        breakpointGutter,
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onChangeRef.current(update.state.doc.toString());
          }
        }),
        EditorView.theme({
          "&": {
            height: "100%",
            fontSize: "13px",
          },
          ".cm-content": {
            fontFamily: '"SF Mono", "Fira Code", "Cascadia Code", monospace',
            lineHeight: "1.6",
            padding: "12px 0",
          },
          ".cm-gutters": {
            background: "var(--card)",
            color: "var(--text-muted)",
            border: "none",
            borderRight: "1px solid var(--border)",
          },
          ".cm-activeLineGutter": {
            background: "var(--border)",
          },
          ".cm-activeLine": {
            background: "rgba(255, 255, 255, 0.03)",
          },
          ".cm-cursor": {
            borderLeftColor: "var(--accent)",
          },
          ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
            background: "rgba(34, 197, 94, 0.15) !important",
          },
          ".cm-lint-marker": {
            fontSize: "12px",
          },
          ".cm-tooltip": {
            background: "var(--card)",
            border: "1px solid var(--border)",
            borderRadius: "6px",
          },
          ".cm-tooltip-autocomplete ul li": {
            padding: "4px 8px",
          },
          ".cm-completionLabel": {
            fontFamily: '"SF Mono", monospace',
            fontSize: "12px",
          },
          ".cm-foldGutter": {
            color: "var(--text-muted)",
          },
        }),
      ],
    });

    const view = new EditorView({
      state,
      parent: containerRef.current,
    });

    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sync external value changes (e.g., switching tabs)
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;

    const currentDoc = view.state.doc.toString();
    if (currentDoc !== value) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: value },
      });
    }
  }, [value]);

  // Listen for goto-line events from diagnostic clicks
  useEffect(() => {
    const handler = (e: Event) => {
      const view = viewRef.current;
      if (!view) return;
      const { line } = (e as CustomEvent).detail;
      const pos = view.state.doc.line(Math.max(1, line)).from;
      view.dispatch({
        selection: { anchor: pos },
        scrollIntoView: true,
      });
      view.focus();
    };
    window.addEventListener("editor-goto", handler);
    return () => window.removeEventListener("editor-goto", handler);
  }, []);

  return <div ref={containerRef} className="st-editor-container" style={{ height: "100%" }} />;
}
