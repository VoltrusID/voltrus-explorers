import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ProProvider } from "@mactools/ui";
import App from "./App";
import "./styles.css";

const PRO_FEATURES = [
  "Virtual serial port (PTY) for loopback tests and bridging",
  "Modbus RTU frame simulation templates",
  "Communication log export to CSV",
];

// Center and size the window on launch
async function initWindow() {
  const win = getCurrentWindow();
  await win.setSize(new (await import("@tauri-apps/api/dpi")).LogicalSize(1100, 720));
  await win.center();
}

initWindow().catch(() => {});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="Serial Monitor" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);
