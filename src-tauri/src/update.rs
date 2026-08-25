use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

const UPDATE_CHECK_DELAY: Duration = Duration::from_secs(12);
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    available: bool,
    current_version: String,
    version: Option<String>,
    notes: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percent: Option<u64>,
}

pub struct UpdateManager {
    operation: tokio::sync::Mutex<()>,
    pending: Mutex<Option<Update>>,
}

impl Default for UpdateManager {
    fn default() -> Self {
        Self {
            operation: tokio::sync::Mutex::new(()),
            pending: Mutex::new(None),
        }
    }
}

#[tauri::command]
pub async fn check_app_update(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, UpdateManager>,
) -> Result<UpdateSnapshot, String> {
    ensure_main_window(&window)?;
    check_for_update(&app, manager.inner(), true).await
}

#[tauri::command]
pub async fn install_app_update(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, UpdateManager>,
) -> Result<(), String> {
    ensure_main_window(&window)?;
    let _operation = manager.operation.lock().await;
    let update = manager
        .lock_pending()
        .clone()
        .ok_or_else(|| "There is no verified update ready to download.".to_string())?;

    let downloaded = Arc::new(AtomicU64::new(0));
    let progress_downloaded = downloaded.clone();
    let progress_app = app.clone();
    let finish_app = app.clone();
    let result = update
        .download_and_install(
            move |chunk, total_bytes| {
                let downloaded_bytes = progress_downloaded
                    .fetch_add(chunk as u64, Ordering::AcqRel)
                    .saturating_add(chunk as u64);
                let percent = total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| downloaded_bytes.saturating_mul(100) / total)
                    .map(|value| value.min(100));
                let _ = progress_app.emit(
                    "update:progress",
                    UpdateProgress {
                        downloaded_bytes,
                        total_bytes,
                        percent,
                    },
                );
            },
            move || {
                let _ = finish_app.emit(
                    "update:progress",
                    UpdateProgress {
                        downloaded_bytes: downloaded.load(Ordering::Acquire),
                        total_bytes: None,
                        percent: Some(100),
                    },
                );
            },
        )
        .await;

    if result.is_err() {
        let message = "The signed update could not be downloaded or installed.";
        let _ = app.emit("update:error", message);
        return Err(message.to_string());
    }

    *manager.lock_pending() = None;

    // Windows installers terminate the old process themselves. Other platforms
    // return after replacement, so explicitly launch the newly installed build.
    #[cfg(not(target_os = "windows"))]
    app.restart();

    #[cfg(target_os = "windows")]
    Ok(())
}

pub fn start_background_check(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(UPDATE_CHECK_DELAY).await;
        if let Some(manager) = app.try_state::<UpdateManager>() {
            let _ = check_for_update(&app, manager.inner(), false).await;
        }
    });
}

async fn check_for_update(
    app: &AppHandle,
    manager: &UpdateManager,
    report_error: bool,
) -> Result<UpdateSnapshot, String> {
    let current_version = app.package_info().version.to_string();
    if !updater_enabled() {
        return Ok(no_update(current_version));
    }

    let _operation = manager.operation.lock().await;
    let check = app
        .updater_builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()
        .map_err(|_| "The updater could not be initialized.".to_string())?
        .check()
        .await;

    let update = match check {
        Ok(update) => update,
        Err(_) => {
            let message = "Could not check for a signed application update.";
            if report_error {
                let _ = app.emit("update:error", message);
            }
            return Err(message.to_string());
        }
    };

    let Some(mut update) = update else {
        *manager.lock_pending() = None;
        return Ok(no_update(current_version));
    };
    update.timeout = Some(UPDATE_DOWNLOAD_TIMEOUT);
    let snapshot = UpdateSnapshot {
        available: true,
        current_version,
        version: Some(update.version.clone()),
        notes: update.body.clone(),
    };
    *manager.lock_pending() = Some(update);
    let _ = app.emit("update:available", snapshot.clone());
    Ok(snapshot)
}

fn updater_enabled() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("ORCESTR_ENABLE_DEBUG_UPDATER").is_some()
}

fn no_update(current_version: String) -> UpdateSnapshot {
    UpdateSnapshot {
        available: false,
        current_version,
        version: None,
        notes: None,
    }
}

fn ensure_main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("This command is not available to the current window".to_string())
    }
}

impl UpdateManager {
    fn lock_pending(&self) -> MutexGuard<'_, Option<Update>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_update_snapshot_preserves_current_version() {
        let snapshot = no_update("0.4.2".to_string());
        assert!(!snapshot.available);
        assert_eq!(snapshot.current_version, "0.4.2");
        assert!(snapshot.version.is_none());
    }
}
