import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Embedded local broker with auth, TLS and WebSocket",
  "More than one saved connection profile",
  "Custom CA, client certificates and Last Will",
  "Hex and Base64 payload decoders",
  "Message history export to CSV",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="MQTT Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);
