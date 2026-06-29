import React from "react";
import ReactDOM from "react-dom/client";
import { ThemeProvider } from "next-themes";
import App from "./App";
import { ToastProvider } from "./components/ToastProvider";
import "./styles/index.css";

// Apply stored theme BEFORE React mounts to avoid a flash of the wrong theme.
// next-themes will take over once the ThemeProvider mounts; this just ensures
// the <html> class is correct on first paint.
try {
  const stored = localStorage.getItem("theme");
  if (stored === "light" || stored === "dark") {
    document.documentElement.classList.remove("light", "dark");
    document.documentElement.classList.add(stored);
  }
} catch {
  /* localStorage unavailable */
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ThemeProvider
      attribute="class"
      defaultTheme="dark"
      enableSystem={false}
      storageKey="theme"
    >
      <ToastProvider>
        <App />
      </ToastProvider>
    </ThemeProvider>
  </React.StrictMode>,
);
