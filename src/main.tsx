import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import App from "./App";
import SpotlightWindow from "./SpotlightWindow";

const label = getCurrentWebviewWindow().label;
const Root = label === "spotlight" ? SpotlightWindow : App;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
