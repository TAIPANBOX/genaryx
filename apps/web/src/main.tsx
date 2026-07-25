// Zero-egress sandbox guard: must be the very first thing this entry point
// runs, before React or `App`, so no other module gets a chance to capture
// an unpatched `fetch`/`XMLHttpRequest`/`WebSocket`/`EventSource`/
// `navigator.credentials` first. Side-effect only; no-ops unless this is the
// `VITE_GENARYX_MOCK` build (see the module's own doc comment).
import "./demo/sandboxGuard";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
