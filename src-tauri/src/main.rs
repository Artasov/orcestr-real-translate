#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod auth;
mod config;
mod realtime;
mod secret_store;
mod update;

use std::sync::{Arc, Mutex};

use auth::{
    open_legal_document, ApiError, AuthManager, AuthMethods, AuthSnapshot, LegalDocument,
    LoginRequest, OAuthProvider, PasswordResetConfirmRequest,
};
use config::{AuthConfig, REDIRECT_URI};
use once_cell::sync::Lazy;
use realtime::{RealtimeChannel, RealtimeManager, RealtimeStartRequest};
use secret_store::{OpenAiKeyStatus, SecretStore};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_deep_link::DeepLinkExt;

static PENDING_DEEP_LINKS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
const MAX_PENDING_DEEP_LINKS: usize = 4;
const MAX_DEEP_LINK_LENGTH: usize = 8 * 1024;

fn ensure_main_window(window: &WebviewWindow) -> Result<(), ApiError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(ApiError::command(
            "This command is not available to the current window.",
        ))
    }
}

#[tauri::command]
fn auth_status(
    window: WebviewWindow,
    manager: State<'_, Arc<AuthManager>>,
) -> Result<AuthSnapshot, ApiError> {
    ensure_main_window(&window)?;
    Ok(manager.snapshot())
}

#[tauri::command]
async fn auth_methods(
    window: WebviewWindow,
    manager: State<'_, Arc<AuthManager>>,
) -> Result<AuthMethods, ApiError> {
    ensure_main_window(&window)?;
    manager.methods().await
}

#[tauri::command]
async fn auth_legal_documents(
    window: WebviewWindow,
    manager: State<'_, Arc<AuthManager>>,
    language: String,
) -> Result<Vec<LegalDocument>, ApiError> {
    ensure_main_window(&window)?;
    manager.legal_documents(&language).await
}

#[tauri::command]
async fn auth_bootstrap(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, Arc<AuthManager>>,
) -> Result<AuthSnapshot, ApiError> {
    ensure_main_window(&window)?;
    let snapshot = manager.bootstrap().await;
    let _ = app.emit("auth:changed", snapshot.clone());
    Ok(snapshot)
}

#[tauri::command]
async fn auth_login(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, Arc<AuthManager>>,
    request: LoginRequest,
) -> Result<serde_json::Value, ApiError> {
    ensure_main_window(&window)?;
    let result = manager.login(request).await;
    let _ = app.emit("auth:changed", manager.snapshot());
    result
}

#[tauri::command]
async fn auth_me(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, Arc<AuthManager>>,
) -> Result<serde_json::Value, ApiError> {
    ensure_main_window(&window)?;
    let result = manager.me().await;
    let _ = app.emit("auth:changed", manager.snapshot());
    result
}

#[tauri::command]
async fn auth_refresh(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, Arc<AuthManager>>,
) -> Result<serde_json::Value, ApiError> {
    ensure_main_window(&window)?;
    let result = manager.refresh().await;
    let _ = app.emit("auth:changed", manager.snapshot());
    result
}

#[tauri::command]
async fn auth_begin_oauth(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, Arc<AuthManager>>,
    provider: OAuthProvider,
) -> Result<AuthSnapshot, ApiError> {
    ensure_main_window(&window)?;
    let result = manager.begin_oauth(&app, provider).await;
    let _ = app.emit("auth:changed", manager.snapshot());
    result
}

#[tauri::command]
async fn auth_cancel_oauth(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, Arc<AuthManager>>,
) -> Result<AuthSnapshot, ApiError> {
    ensure_main_window(&window)?;
    let snapshot = manager.cancel_oauth().await;
    let _ = app.emit("auth:changed", snapshot.clone());
    Ok(snapshot)
}

#[tauri::command]
async fn auth_logout(
    window: WebviewWindow,
    app: AppHandle,
    manager: State<'_, Arc<AuthManager>>,
    realtime: State<'_, Arc<RealtimeManager>>,
) -> Result<(), ApiError> {
    ensure_main_window(&window)?;
    realtime.stop_all(&app).await;
    let result = manager.logout().await;
    let _ = app.emit("auth:changed", manager.snapshot());
    result
}

#[tauri::command]
async fn auth_password_reset_request(
    window: WebviewWindow,
    manager: State<'_, Arc<AuthManager>>,
    email: String,
) -> Result<(), ApiError> {
    ensure_main_window(&window)?;
    manager.request_password_reset(&email).await
}

#[tauri::command]
async fn auth_password_reset_confirm(
    window: WebviewWindow,
    manager: State<'_, Arc<AuthManager>>,
    request: PasswordResetConfirmRequest,
) -> Result<(), ApiError> {
    ensure_main_window(&window)?;
    manager.confirm_password_reset(request).await
}

#[tauri::command]
fn auth_open_legal_document(
    window: WebviewWindow,
    app: AppHandle,
    url: String,
) -> Result<(), ApiError> {
    ensure_main_window(&window)?;
    open_legal_document(&app, &url)
}

#[tauri::command]
async fn openai_key_status(
    window: WebviewWindow,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<OpenAiKeyStatus, String> {
    ensure_main_window(&window).map_err(|error| error.message)?;
    secrets.openai_key_status().await
}

#[tauri::command]
async fn openai_key_save(
    window: WebviewWindow,
    app: AppHandle,
    secrets: State<'_, Arc<SecretStore>>,
    realtime: State<'_, Arc<RealtimeManager>>,
    api_key: String,
) -> Result<OpenAiKeyStatus, String> {
    ensure_main_window(&window).map_err(|error| error.message)?;
    // A key change is an explicit session boundary: no live connection keeps
    // using a credential that Settings has just replaced.
    realtime.stop_all(&app).await;
    secrets.set_openai_api_key(&api_key).await
}

#[tauri::command]
async fn openai_key_delete(
    window: WebviewWindow,
    app: AppHandle,
    secrets: State<'_, Arc<SecretStore>>,
    realtime: State<'_, Arc<RealtimeManager>>,
) -> Result<OpenAiKeyStatus, String> {
    ensure_main_window(&window).map_err(|error| error.message)?;
    realtime.stop_all(&app).await;
    secrets.clear_openai_api_key().await
}

#[tauri::command]
fn audio_list_devices(window: WebviewWindow) -> Result<audio::AudioDeviceInventory, String> {
    ensure_main_window(&window).map_err(|error| error.message)?;
    audio::list_audio_devices()
}

#[tauri::command]
async fn realtime_start(
    window: WebviewWindow,
    app: AppHandle,
    secrets: State<'_, Arc<SecretStore>>,
    realtime: State<'_, Arc<RealtimeManager>>,
    request: RealtimeStartRequest,
) -> Result<(), String> {
    ensure_main_window(&window).map_err(|error| error.message)?;
    realtime
        .inner()
        .clone()
        .start(app, secrets.inner().clone(), request)
        .await
}

#[tauri::command]
async fn realtime_stop(
    window: WebviewWindow,
    app: AppHandle,
    realtime: State<'_, Arc<RealtimeManager>>,
    channel: RealtimeChannel,
) -> Result<(), String> {
    ensure_main_window(&window).map_err(|error| error.message)?;
    realtime.stop(&app, channel).await;
    Ok(())
}

#[tauri::command]
async fn realtime_set_playback_enabled(
    window: WebviewWindow,
    realtime: State<'_, Arc<RealtimeManager>>,
    channel: RealtimeChannel,
    enabled: bool,
) -> Result<(), String> {
    ensure_main_window(&window).map_err(|error| error.message)?;
    realtime.set_playback_enabled(channel, enabled).await
}

#[tauri::command]
async fn realtime_stop_all(
    window: WebviewWindow,
    app: AppHandle,
    realtime: State<'_, Arc<RealtimeManager>>,
) -> Result<(), String> {
    ensure_main_window(&window).map_err(|error| error.message)?;
    realtime.stop_all(&app).await;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        // This plugin runs first so a second process forwards a callback without
        // initializing another browser window or credential store.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(value) = args
                .into_iter()
                .find(|argument| argument.starts_with(REDIRECT_URI))
            {
                queue_or_dispatch_deep_link(app, value);
            } else {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let config = AuthConfig::load().map_err(std::io::Error::other)?;
            let manager = Arc::new(AuthManager::new(config).map_err(std::io::Error::other)?);
            app.manage(manager.clone());
            app.manage(Arc::new(SecretStore::new()));
            app.manage(Arc::new(RealtimeManager::new()));
            app.manage(update::UpdateManager::default());

            // Runtime registration supports local development on Windows/Linux;
            // packaged macOS builds receive the scheme from the bundle metadata.
            let _ = app.deep_link().register_all();
            setup_deep_link_listener(app.handle(), manager.clone());
            flush_pending_deep_links(app.handle(), manager);
            update::start_background_check(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            auth_status,
            auth_methods,
            auth_legal_documents,
            auth_bootstrap,
            auth_login,
            auth_me,
            auth_refresh,
            auth_begin_oauth,
            auth_cancel_oauth,
            auth_logout,
            auth_password_reset_request,
            auth_password_reset_confirm,
            auth_open_legal_document,
            openai_key_status,
            openai_key_save,
            openai_key_delete,
            audio_list_devices,
            realtime_start,
            realtime_set_playback_enabled,
            realtime_stop,
            realtime_stop_all,
            update::check_app_update,
            update::install_app_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Orcestr Real Translate");
}

fn queue_or_dispatch_deep_link(app: &AppHandle, value: String) {
    if value.len() > MAX_DEEP_LINK_LENGTH || !value.starts_with(REDIRECT_URI) {
        return;
    }
    if let Some(manager) = app.try_state::<Arc<AuthManager>>() {
        dispatch_deep_link(app, manager.inner().clone(), value);
        return;
    }

    let mut pending = PENDING_DEEP_LINKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if pending.len() == MAX_PENDING_DEEP_LINKS {
        pending.remove(0);
    }
    pending.push(value);
}

fn flush_pending_deep_links(app: &AppHandle, manager: Arc<AuthManager>) {
    let pending = {
        let mut pending = PENDING_DEEP_LINKS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.drain(..).collect::<Vec<_>>()
    };
    for value in pending {
        dispatch_deep_link(app, manager.clone(), value);
    }
}

fn setup_deep_link_listener(app: &AppHandle, manager: Arc<AuthManager>) {
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in urls {
            let value = url.to_string();
            if value.starts_with(REDIRECT_URI) {
                dispatch_deep_link(app, manager.clone(), value);
            }
        }
    }

    let app_handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            let value = url.to_string();
            if value.starts_with(REDIRECT_URI) {
                dispatch_deep_link(&app_handle, manager.clone(), value);
            }
        }
    });
}

fn dispatch_deep_link(app: &AppHandle, manager: Arc<AuthManager>, value: String) {
    if value.len() > MAX_DEEP_LINK_LENGTH {
        return;
    }
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        manager.handle_callback(&app_handle, &value).await;
        show_main_window(&app_handle);
    });
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
