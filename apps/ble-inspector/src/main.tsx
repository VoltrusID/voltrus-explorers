import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Write characteristic values",
  "Subscribe to notifications and log them",
  "Export the communication log to CSV",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="BLE Inspector" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);
