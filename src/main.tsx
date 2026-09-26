import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { i18nReady } from "./i18n";
import { logStartupEvent } from "./lib/tauri";
import "./index.css";
import App from "./App.tsx";

// E2E tests only: answer every backend call from a fake. Vite drops this from other builds.
if (import.meta.env.MODE === "e2e") await import("../e2e/fake-backend");

await i18nReady;
logStartupEvent("i18n_ready", performance.now()).catch(() => {});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>
);
logStartupEvent("root_rendered", performance.now()).catch(() => {});
