use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tauri::{AppHandle, Manager, State, WebviewWindow};

const LOG_FILE_NAME: &str = "orcestr-real-translate.log";
const MAX_FIELD_LENGTH: usize = 8 * 1024;

#[derive(Clone)]
pub(crate) struct DiagnosticsLog {
    path: PathBuf,
    file: Arc<Mutex<File>>,
}

impl DiagnosticsLog {
    pub(crate) fn initialize(app: &AppHandle) -> io::Result<Self> {
        let directory = app.path().app_log_dir().map_err(io::Error::other)?;
        fs::create_dir_all(&directory)?;
        let path = directory.join(LOG_FILE_NAME);
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;
        let log = Self {
            path,
            file: Arc::new(Mutex::new(file)),
        };
        log.write("INFO", "Native application startup began");
        Ok(log)
    }

    pub(crate) fn write(&self, level: &str, message: &str) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let message = sanitize(message);
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "[{timestamp}] {level} {message}");
            let _ = file.flush();
        }
    }

    fn display_path(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RendererDiagnosticEvent {
    level: String,
    message: String,
    stack: Option<String>,
    source: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
}

#[tauri::command]
pub(crate) fn diagnostics_log_renderer(
    window: WebviewWindow,
    diagnostics: State<'_, DiagnosticsLog>,
    event: RendererDiagnosticEvent,
) -> Result<(), String> {
    ensure_main_window(&window)?;
    let level = match event.level.as_str() {
        "info" => "INFO",
        "warning" => "WARN",
        "error" => "ERROR",
        _ => return Err("Unsupported diagnostics level".to_string()),
    };
    let mut details = format!("Renderer: {}", bounded(&event.message));
    if let Some(source) = event.source.filter(|value| !value.is_empty()) {
        details.push_str(&format!(
            " | source={}{}{}",
            bounded(&source),
            event
                .line
                .map(|line| format!(":{line}"))
                .unwrap_or_default(),
            event
                .column
                .map(|column| format!(":{column}"))
                .unwrap_or_default()
        ));
    }
    if let Some(stack) = event.stack.filter(|value| !value.is_empty()) {
        details.push_str(" | stack=");
        details.push_str(&bounded(&stack));
    }
    diagnostics.write(level, &details);
    Ok(())
}

#[tauri::command]
pub(crate) fn diagnostics_log_path(
    window: WebviewWindow,
    diagnostics: State<'_, DiagnosticsLog>,
) -> Result<String, String> {
    ensure_main_window(&window)?;
    Ok(diagnostics.display_path())
}

#[tauri::command]
pub(crate) fn diagnostics_open_devtools(window: WebviewWindow) -> Result<(), String> {
    ensure_main_window(&window)?;
    window.open_devtools();
    Ok(())
}

fn ensure_main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Diagnostics are not available to the current window".to_string())
    }
}

fn bounded(value: &str) -> String {
    value.chars().take(MAX_FIELD_LENGTH).collect()
}

fn sanitize(value: &str) -> String {
    bounded(value).replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_fields_are_bounded_and_single_line() {
        assert_eq!(sanitize("first\r\nsecond"), "first  second");
        assert_eq!(
            bounded(&"x".repeat(MAX_FIELD_LENGTH + 4)).len(),
            MAX_FIELD_LENGTH
        );
    }
}
