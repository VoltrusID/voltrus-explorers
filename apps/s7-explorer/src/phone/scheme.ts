import { useSyncExternalStore } from "react";

const QUERY = "(prefers-color-scheme: dark)";

function subscribe(listener: () => void) {
  const mq = window.matchMedia(QUERY);
  mq.addEventListener("change", listener);
  return () => mq.removeEventListener("change", listener);
}

/** The system colour scheme. A phone follows it (iOS draws the status bar from it) and the chart reads it. */
export function useSystemScheme(): "dark" | "light" {
  return useSyncExternalStore(
    subscribe,
    () => (window.matchMedia(QUERY).matches ? "dark" : "light"),
    () => "dark",
  );
}
