import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "JetStream stream and consumer management",
  "KV store browser with live watch",
  "Object store browser, upload and download",
  "$SYS server monitor",
  "Protobuf schema decoding",
  "CSV export of captured messages",
  "More than one saved connection profile",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="NATS Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);
