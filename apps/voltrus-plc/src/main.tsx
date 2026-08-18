import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
import { setupRuntimeListeners } from "./store";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// Only setup Tauri listeners when running inside Tauri webview
if ((window as any).__TAURI_INTERNALS__) {
  setupRuntimeListeners();
}
