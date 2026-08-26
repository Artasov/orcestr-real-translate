import { invoke } from "@tauri-apps/api/core";

interface RendererDiagnosticEvent {
  level: "error" | "warning" | "info";
  message: string;
  stack?: string;
  source?: string;
  line?: number;
  column?: number;
}

let fatalScreenVisible = false;

export function installRendererDiagnostics(): void {
  window.addEventListener("error", (event) => {
    reportRendererEvent({
      level: "error",
      message: event.message || "Uncaught renderer error",
      stack: event.error instanceof Error ? event.error.stack : undefined,
      source: event.filename || undefined,
      line: event.lineno || undefined,
      column: event.colno || undefined,
    });
    showFatalBootstrapError(event.message || "Uncaught renderer error");
  });

  window.addEventListener("unhandledrejection", (event) => {
    const error = event.reason;
    reportRendererEvent({
      level: "error",
      message: error instanceof Error ? error.message : String(error),
      stack: error instanceof Error ? error.stack : undefined,
    });
    showFatalBootstrapError(
      error instanceof Error ? error.message : "Unhandled renderer rejection",
    );
  });

  window.addEventListener("keydown", (event) => {
    const inspectorShortcut =
      event.key === "F12" ||
      (((event.ctrlKey && event.shiftKey) || (event.metaKey && event.altKey)) &&
        event.key.toLowerCase() === "i");
    if (!inspectorShortcut) return;
    event.preventDefault();
    void invoke("diagnostics_open_devtools");
  });

  reportRendererEvent({ level: "info", message: "Renderer bootstrap began" });
}

function reportRendererEvent(event: RendererDiagnosticEvent): void {
  void invoke("diagnostics_log_renderer", { event }).catch(() => undefined);
}

function showFatalBootstrapError(message: string): void {
  window.setTimeout(() => {
    const root = document.getElementById("root");
    if (!root || root.childElementCount > 0 || fatalScreenVisible) return;
    fatalScreenVisible = true;

    const surface = document.createElement("section");
    surface.className = "fatal-bootstrap";
    surface.setAttribute("role", "alert");

    const title = document.createElement("h1");
    title.textContent = "Orcestr Real Translate could not start";
    const description = document.createElement("p");
    description.textContent = message;
    const hint = document.createElement("p");
    hint.textContent = "Press F12 or Ctrl+Shift+I to open DevTools.";
    const path = document.createElement("code");
    path.textContent = "Loading diagnostics log path…";

    surface.append(title, description, hint, path);
    root.replaceChildren(surface);

    void invoke<string>("diagnostics_log_path")
      .then((value) => {
        path.textContent = value;
      })
      .catch(() => {
        path.textContent = "Diagnostics log path is unavailable.";
      });
  }, 0);
}
