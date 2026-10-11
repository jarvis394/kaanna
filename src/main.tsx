import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

async function boot() {
  // Opened in a plain browser during development: use a fake backend.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installDemoBackend } = await import("./lib/demo");
    installDemoBackend();
    document.documentElement.dataset.demo = "true";
  }
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

boot();
