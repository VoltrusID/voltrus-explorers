import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { DevicePanel } from "./components/DevicePanel";
import "./styles.css";

// Tiny hash router: a dedicated Tauri window loads `index.html#/sim` to render
// the device control panel instead of the full app.
const route = typeof window !== "undefined" ? window.location.hash : "";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {route === "#/sim" ? <DevicePanel /> : <App />}
  </React.StrictMode>,
);
