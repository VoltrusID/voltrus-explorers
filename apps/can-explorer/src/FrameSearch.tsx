import { useSyncExternalStore } from "react";

/** Name search and data pattern, shared by the Monitor and the Trace and kept while switching tabs. */
export interface SearchState {
  text: string;
  data: string;
}

let state: SearchState = { text: "", data: "" };
const listeners = new Set<() => void>();

function setSearch(patch: Partial<SearchState>) {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export function useFrameSearch(): SearchState {
  return useSyncExternalStore(
    (l) => { listeners.add(l); return () => { listeners.delete(l); }; },
    () => state,
    () => state,
  );
}

/** The two inputs, for a view's toolbar. */
export function FrameSearchInputs({ value }: { value: SearchState }) {
  return (
    <>
      <input
        className="vt-input"
        aria-label="Search names"
        placeholder="Search names"
        title="Words in the DBC message name or the protocol label; ! in front hides."
        value={value.text}
        spellCheck={false}
        onChange={(e) => setSearch({ text: e.target.value })}
        style={{ width: 150 }}
      />
      <input
        className="vt-input vt-input--mono"
        aria-label="Data pattern"
        placeholder="Data: xx 12 xx"
        title="Data bytes from the first one, two hex digits each; x stands for any digit (xx 12 xx, 01 ?F)."
        value={value.data}
        spellCheck={false}
        onChange={(e) => setSearch({ data: e.target.value })}
        style={{ width: 130 }}
      />
    </>
  );
}
