use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reqwest::redirect::Policy;
use reqwest::{Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::config::{AuthConfig, CLIENT_ID, REDIRECT_URI, SCOPE};
use crate::secret_store::{SecretStore, StoredSession, StoredSessionKind};

const ATTEMPT_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MAX_AUTHORIZATION_URL_LENGTH: usize = 8 * 1024;
const MAX_BROWSER_LOGIN_URL_LENGTH: usize = 8 * 1024;
const MAX_AUTH_CODE_LENGTH: usize = 4 * 1024;
const MAX_TOKEN_LENGTH: usize = 24 * 1024;
const MAX_RESPONSE_LENGTH: u64 = 256 * 1024;
const MAX_LEGAL_DOCUMENTS: usize = 16;
const MAX_ERROR_CODE_LENGTH: usize = 128;
const MAX_ERROR_MESSAGE_LENGTH: usize = 2 * 1024;
const MAX_ERROR_REQUEST_ID_LENGTH: usize = 256;
const MAX_ERROR_FIELDS: usize = 64;
const MAX_ERROR_PATH_PARTS: usize = 16;
const MAX_ERROR_PATH_STRING_LENGTH: usize = 128;
const MAX_ERROR_PARAMS: usize = 32;
const MAX_ERROR_PARAM_KEY_LENGTH: usize = 128;
const MAX_ERROR_PARAM_STRING_LENGTH: usize = 512;
const MAX_ERROR_DETAILS_DEPTH: usize = 4;
const MAX_ERROR_DETAILS_NODES: usize = 256;
const MAX_SAFE_JS_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OAuthProvider {
    Google,
    Github,
    Yandex,
}

impl OAuthProvider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Github => "github",
            Self::Yandex => "yandex",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSnapshot {
    phase: &'static str,
    profile: Option<Value>,
    message: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRequest {
    username: String,
    password: String,
    #[serde(default)]
    accepted_legal_documents: Vec<LegalAcceptance>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegalAcceptance {
    document_slug: String,
    version: String,
    language: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasswordResetConfirmRequest {
    email: String,
    code: String,
    password: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuthMethods {
    email_password_allowed: bool,
    allowed_oauth_providers: Vec<String>,
    oauth_client_ids: BTreeMap<String, String>,
    country_known: bool,
    allowed_email_domains: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalDocument {
    document_slug: String,
    title: String,
    version: String,
    language: String,
    required_for_registration: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiFieldError {
    path: Vec<ApiFieldPathPart>,
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<BTreeMap<String, ApiParamValue>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum ApiFieldPathPart {
    String(String),
    Number(i64),
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum ApiParamValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub fields: Vec<ApiFieldError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<BTreeMap<String, ApiParamValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl ApiError {
    fn new(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            fields: Vec::new(),
            params: None,
            request_id: None,
        }
    }

    pub fn command(message: impl Into<String>) -> Self {
        Self::new(400, "invalid_native_request", message)
    }

    fn network() -> Self {
        Self::new(
            0,
            "network_error",
            "Could not reach Orcestr. Check your connection and try again.",
        )
    }

    fn invalid_response() -> Self {
        Self::new(
            502,
            "invalid_error_response",
            "Orcestr returned an invalid authentication response.",
        )
    }

    fn not_authenticated() -> Self {
        Self::new(401, "not_authenticated", "Sign in to continue.")
    }

    fn credential_store(message: impl Into<String>) -> Self {
        Self::new(500, "credential_store_unavailable", message)
    }

    fn invalidates_refresh_token(&self) -> bool {
        self.status == 401
            || matches!(
                self.code.as_str(),
                "invalid_grant"
                    | "invalid_token"
                    | "refresh_token_invalid"
                    | "refresh_token_reused"
                    | "refresh_token_expired"
            )
    }
}

#[derive(Debug)]
struct AuthAttempt {
    state: String,
    code_verifier: String,
    created_at: Instant,
}

#[derive(Clone, Debug)]
struct Session {
    // Access tokens intentionally exist only in process memory.
    access_token: String,
    kind: StoredSessionKind,
    profile: Value,
}

#[derive(Debug, Default)]
struct AuthState {
    attempt: Option<AuthAttempt>,
    session: Option<Session>,
    bootstrapping: bool,
    last_error: Option<String>,
}

pub struct AuthManager {
    config: AuthConfig,
    client: reqwest::Client,
    secrets: SecretStore,
    state: Mutex<AuthState>,
    operation: tokio::sync::Mutex<()>,
}

pub fn open_legal_document(app: &AppHandle, value: &str) -> Result<(), ApiError> {
    let url = validate_legal_document_url(value)?;
    app.opener().open_url(url, None::<String>).map_err(|_| {
        ApiError::new(
            500,
            "browser_open_failed",
            "Could not open the Orcestr legal document.",
        )
    })
}

impl AuthManager {
    pub fn new(config: AuthConfig) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!(
                "Orcestr-Real-Translate/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|error| format!("Could not initialize the HTTP client: {error}"))?;

        Ok(Self {
            config,
            client,
            secrets: SecretStore::new(),
            state: Mutex::new(AuthState::default()),
            operation: tokio::sync::Mutex::new(()),
        })
    }

    pub fn snapshot(&self) -> AuthSnapshot {
        snapshot_from_state(&self.lock_state())
    }

    pub async fn methods(&self) -> Result<AuthMethods, ApiError> {
        let wire: MethodsResponse = self
            .get_json(self.client.get(self.config.methods_endpoint()))
            .await?;
        wire.validate()
    }

    pub async fn legal_documents(&self, language: &str) -> Result<Vec<LegalDocument>, ApiError> {
        if !matches!(language, "en" | "ru") {
            return Err(ApiError::command("Legal document language is invalid."));
        }
        let wire: Vec<LegalDocumentResponse> = self
            .get_json(
                self.client
                    .get(self.config.legal_documents_endpoint(language)),
            )
            .await?;
        if wire.len() > MAX_LEGAL_DOCUMENTS {
            return Err(ApiError::invalid_response());
        }
        wire.into_iter()
            .map(LegalDocumentResponse::validate)
            .collect()
    }

    pub async fn login(&self, request: LoginRequest) -> Result<Value, ApiError> {
        validate_login(&request)?;
        let _operation = self.operation.lock().await;
        let payload = json!({
            "username": request.username,
            "password": request.password,
            "accepted_legal_documents": request.accepted_legal_documents.into_iter().map(|item| json!({
                "document_slug": item.document_slug,
                "version": item.version,
                "language": item.language,
            })).collect::<Vec<_>>(),
        });
        let tokens: NativeTokenResponse = self
            .get_json(
                self.client
                    .post(self.config.native_login_endpoint())
                    .json(&payload),
            )
            .await?;
        let tokens = tokens.validate()?;
        self.accept_tokens(StoredSessionKind::Password, tokens)
            .await
    }

    pub async fn me(&self) -> Result<Value, ApiError> {
        let _operation = self.operation.lock().await;
        let session = match {
            let state = self.lock_state();
            session_for_me(&state)
        }? {
            Some(session) => session,
            None => return self.restore_locked().await,
        };
        match self
            .fetch_profile(session.kind, &session.access_token)
            .await
        {
            Ok(profile) => {
                self.set_authenticated(session.kind, session.access_token, profile.clone());
                Ok(profile)
            }
            Err(error) if error.status == 401 => self.refresh_locked().await,
            Err(error) => Err(error),
        }
    }

    pub async fn refresh(&self) -> Result<Value, ApiError> {
        let _operation = self.operation.lock().await;
        self.refresh_locked().await
    }

    pub async fn bootstrap(&self) -> AuthSnapshot {
        let _operation = self.operation.lock().await;
        {
            let mut state = self.lock_state();
            state.attempt = None;
            state.session = None;
            state.bootstrapping = true;
            state.last_error = None;
        }
        match self.restore_locked().await {
            Ok(_) => {}
            Err(error) if error.code == "not_authenticated" => self.set_signed_out(),
            Err(error) => self.fail(&error.message),
        }
        self.snapshot()
    }

    pub async fn logout(&self) -> Result<(), ApiError> {
        let _operation = self.operation.lock().await;
        let stored = self.secrets.get_session().await.ok().flatten();
        let active = self.lock_state().session.clone();

        if let Some(stored) = stored.as_ref() {
            match stored.kind {
                StoredSessionKind::Password => {
                    let _ = self.logout_native(stored, active.as_ref()).await;
                }
                StoredSessionKind::OAuth2 => {
                    let _ = self.revoke_oauth_refresh_token(&stored.refresh_token).await;
                }
            }
        }

        let clear_result = self.secrets.clear_session().await;
        let mut state = self.lock_state();
        apply_logout_clear_result(&mut state, clear_result)
    }

    pub async fn request_password_reset(&self, email: &str) -> Result<(), ApiError> {
        validate_email(email)?;
        self.expect_success(
            self.client
                .post(self.config.password_reset_request_endpoint())
                .json(&json!({"email": email})),
        )
        .await
    }

    pub async fn confirm_password_reset(
        &self,
        request: PasswordResetConfirmRequest,
    ) -> Result<(), ApiError> {
        validate_password_reset_confirm(&request)?;
        self.expect_success(
            self.client
                .post(self.config.password_reset_confirm_endpoint())
                .json(&json!({
                    "email": request.email,
                    "code": request.code,
                    "password": request.password,
                })),
        )
        .await
    }

    pub async fn begin_oauth(
        &self,
        app: &AppHandle,
        provider: OAuthProvider,
    ) -> Result<AuthSnapshot, ApiError> {
        let _operation = self.operation.lock().await;
        ensure_oauth_start_allowed(&self.lock_state())?;
        let state = random_base64_url(32)?;
        let code_verifier = random_base64_url(48)?;
        let code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
        let browser_login_url =
            build_oauth_browser_login_url(&self.config, provider, &state, &code_challenge)?;
        {
            let mut current = self.lock_state();
            current.attempt = Some(AuthAttempt {
                state,
                code_verifier,
                created_at: Instant::now(),
            });
            current.session = None;
            current.bootstrapping = false;
            current.last_error = None;
        }

        if app
            .opener()
            .open_url(browser_login_url, None::<String>)
            .is_err()
        {
            self.fail("Could not open the secure Orcestr sign-in page.");
            return Err(ApiError::new(
                500,
                "browser_open_failed",
                "Could not open the secure Orcestr sign-in page.",
            ));
        }
        Ok(self.snapshot())
    }

    pub async fn cancel_oauth(&self) -> AuthSnapshot {
        let _operation = self.operation.lock().await;
        {
            let mut state = self.lock_state();
            clear_oauth_attempt(&mut state);
        }
        self.snapshot()
    }

    pub async fn handle_callback(&self, app: &AppHandle, callback_url: &str) {
        let _operation = self.operation.lock().await;
        let callback = match parse_callback(callback_url) {
            Ok(callback) => callback,
            Err(_) => return,
        };
        let attempt = match {
            let mut state = self.lock_state();
            take_callback_attempt(&mut state, callback.state())
        } {
            CallbackAttempt::Ignored => return,
            CallbackAttempt::Expired => {
                self.emit_snapshot(app);
                return;
            }
            CallbackAttempt::Active(attempt) => attempt,
        };

        match callback {
            ParsedCallback::Denied { .. } => {
                self.fail("Authorization was cancelled or denied.");
            }
            ParsedCallback::Success { code, .. } => {
                match self
                    .exchange_authorization_code(&code, &attempt.code_verifier)
                    .await
                {
                    Ok(tokens) => {
                        if let Err(error) =
                            self.accept_tokens(StoredSessionKind::OAuth2, tokens).await
                        {
                            self.fail(&error.message);
                        }
                    }
                    Err(error) => self.fail(&error.message),
                }
            }
        }
        self.emit_snapshot(app);
    }

    fn emit_snapshot(&self, app: &AppHandle) {
        let _ = app.emit("auth:changed", self.snapshot());
    }

    async fn restore_locked(&self) -> Result<Value, ApiError> {
        let stored = match self.secrets.get_session().await {
            Ok(Some(stored)) => stored,
            Ok(None) => {
                self.set_signed_out();
                return Err(ApiError::not_authenticated());
            }
            Err(_) => {
                self.set_signed_out();
                return Err(ApiError::credential_store(
                    "The saved session could not be read from the OS credential store.",
                ));
            }
        };
        self.refresh_stored(stored).await
    }

    async fn refresh_locked(&self) -> Result<Value, ApiError> {
        let stored = match self.secrets.get_session().await.map_err(|_| {
            ApiError::credential_store(
                "The saved session could not be read from the OS credential store.",
            )
        })? {
            Some(stored) => stored,
            None => {
                self.set_signed_out();
                return Err(ApiError::not_authenticated());
            }
        };
        self.refresh_stored(stored).await
    }

    async fn refresh_stored(&self, stored: StoredSession) -> Result<Value, ApiError> {
        let result = match stored.kind {
            StoredSessionKind::Password => self.refresh_native(&stored.refresh_token).await,
            StoredSessionKind::OAuth2 => self.refresh_oauth(&stored.refresh_token).await,
        };
        let tokens = match result {
            Ok(tokens) => tokens,
            Err(error) if error.invalidates_refresh_token() => {
                let _ = self.secrets.clear_session().await;
                self.set_signed_out();
                return Err(ApiError::not_authenticated());
            }
            Err(error) => return Err(error),
        };
        self.accept_tokens(stored.kind, tokens).await
    }

    async fn accept_tokens(
        &self,
        kind: StoredSessionKind,
        tokens: TokenPair,
    ) -> Result<Value, ApiError> {
        let stored = StoredSession::new(kind, tokens.refresh_token)
            .map_err(|_| ApiError::invalid_response())?;
        let profile = persist_then_fetch_profile(
            || async {
                self.secrets.set_session(&stored).await.map_err(|_| {
                    ApiError::credential_store("The renewed session could not be stored securely.")
                })
            },
            || self.fetch_profile(kind, &tokens.access_token),
        )
        .await;
        let profile = match profile {
            Ok(profile) => profile,
            Err(error) if error.status == 401 => {
                let _ = self.secrets.clear_session().await;
                self.set_signed_out();
                return Err(ApiError::not_authenticated());
            }
            Err(error) => {
                // The rotated refresh token is already durable. A later me() call
                // can safely retry without replaying the now-invalid predecessor.
                self.set_signed_out();
                return Err(error);
            }
        };
        self.set_authenticated(kind, tokens.access_token, profile.clone());
        Ok(profile)
    }

    async fn refresh_native(&self, refresh_token: &str) -> Result<TokenPair, ApiError> {
        let response: NativeTokenResponse = self
            .get_json(
                self.client
                    .post(self.config.native_refresh_endpoint())
                    .json(&json!({"refresh_token": refresh_token})),
            )
            .await?;
        response.validate()
    }

    async fn exchange_authorization_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<TokenPair, ApiError> {
        self.request_oauth_tokens(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", code_verifier),
        ])
        .await
    }

    async fn refresh_oauth(&self, refresh_token: &str) -> Result<TokenPair, ApiError> {
        self.request_oauth_tokens(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ])
        .await
    }

    async fn request_oauth_tokens(&self, form: &[(&str, &str)]) -> Result<TokenPair, ApiError> {
        let response = self
            .client
            .post(self.config.oauth_token_endpoint())
            .header(reqwest::header::ACCEPT, "application/json")
            .form(form)
            .send()
            .await
            .map_err(|_| ApiError::network())?;
        let status = response.status();
        let bytes = bounded_body(response).await?;
        if !status.is_success() {
            if let Ok(error) = serde_json::from_slice::<OAuthErrorResponse>(&bytes) {
                let valid = valid_error_code(&error.error)
                    && error
                        .error_description
                        .as_deref()
                        .is_none_or(|message| valid_error_text(message, MAX_ERROR_MESSAGE_LENGTH));
                if valid {
                    return Err(ApiError::new(
                        status.as_u16(),
                        error.error,
                        error
                            .error_description
                            .unwrap_or_else(|| "OAuth authorization was rejected.".to_string()),
                    ));
                }
            }
            return Err(parse_api_error(status, &bytes));
        }
        let response: OAuthTokenResponse =
            serde_json::from_slice(&bytes).map_err(|_| ApiError::invalid_response())?;
        response.validate()
    }

    async fn fetch_profile(
        &self,
        kind: StoredSessionKind,
        access_token: &str,
    ) -> Result<Value, ApiError> {
        let endpoint = match kind {
            StoredSessionKind::Password => self.config.native_me_endpoint(),
            StoredSessionKind::OAuth2 => self.config.oauth_userinfo_endpoint(),
        };
        let profile: Value = self
            .get_json(self.client.get(endpoint).bearer_auth(access_token))
            .await?;
        normalize_profile(kind, profile)
    }

    async fn logout_native(
        &self,
        stored: &StoredSession,
        active: Option<&Session>,
    ) -> Result<(), ApiError> {
        let mut request = self
            .client
            .post(self.config.native_logout_endpoint())
            .json(&json!({"refresh_token": stored.refresh_token}));
        if let Some(session) = active.filter(|session| session.kind == StoredSessionKind::Password)
        {
            request = request.bearer_auth(&session.access_token);
        }
        self.expect_success(request).await
    }

    async fn revoke_oauth_refresh_token(&self, refresh_token: &str) -> Result<(), ApiError> {
        self.expect_success(
            self.client
                .post(self.config.oauth_revoke_endpoint())
                .form(&[
                    ("client_id", CLIENT_ID),
                    ("token", refresh_token),
                    ("token_type_hint", "refresh_token"),
                ]),
        )
        .await
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ApiError> {
        let response = request
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| ApiError::network())?;
        let status = response.status();
        let bytes = bounded_body(response).await?;
        if !status.is_success() {
            return Err(parse_api_error(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|_| ApiError::invalid_response())
    }

    async fn expect_success(&self, request: reqwest::RequestBuilder) -> Result<(), ApiError> {
        let response = request
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| ApiError::network())?;
        let status = response.status();
        let bytes = bounded_body(response).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(parse_api_error(status, &bytes))
        }
    }

    fn set_authenticated(&self, kind: StoredSessionKind, access_token: String, profile: Value) {
        let mut state = self.lock_state();
        state.attempt = None;
        state.bootstrapping = false;
        state.last_error = None;
        state.session = Some(Session {
            access_token,
            kind,
            profile,
        });
    }

    fn set_signed_out(&self) {
        let mut state = self.lock_state();
        state.attempt = None;
        state.session = None;
        state.bootstrapping = false;
        state.last_error = None;
    }

    fn fail(&self, message: &str) {
        let mut state = self.lock_state();
        state.attempt = None;
        state.session = None;
        state.bootstrapping = false;
        state.last_error = Some(message.to_string());
    }

    fn lock_state(&self) -> MutexGuard<'_, AuthState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

enum CallbackAttempt {
    Ignored,
    Expired,
    Active(AuthAttempt),
}

fn ensure_oauth_start_allowed(state: &AuthState) -> Result<(), ApiError> {
    if state.attempt.is_some() {
        return Err(ApiError::new(
            409,
            "oauth_authorization_in_progress",
            "Cancel or retry the current authorization attempt first.",
        ));
    }
    Ok(())
}

fn clear_oauth_attempt(state: &mut AuthState) {
    if state.session.is_none() {
        state.attempt = None;
        state.bootstrapping = false;
        state.last_error = None;
    }
}

fn session_for_me(state: &AuthState) -> Result<Option<Session>, ApiError> {
    if let Some(session) = &state.session {
        return Ok(Some(session.clone()));
    }
    if state.attempt.is_some() {
        // Window focus can refetch the current-user query while the system
        // browser owns an OAuth authorization. Treat it as unauthenticated,
        // but keep the PKCE attempt so the incoming deep link remains valid.
        return Err(ApiError::not_authenticated());
    }
    Ok(None)
}

fn take_callback_attempt(state: &mut AuthState, callback_state: &str) -> CallbackAttempt {
    let Some(attempt) = state.attempt.as_ref() else {
        return CallbackAttempt::Ignored;
    };
    if attempt.state != callback_state {
        return CallbackAttempt::Ignored;
    }
    if attempt.created_at.elapsed() > ATTEMPT_LIFETIME {
        state.attempt = None;
        state.session = None;
        state.bootstrapping = false;
        state.last_error = Some("Authorization session expired. Try again.".to_string());
        return CallbackAttempt::Expired;
    }
    CallbackAttempt::Active(
        state
            .attempt
            .take()
            .expect("matching authorization attempt must still exist"),
    )
}

fn apply_logout_clear_result(
    state: &mut AuthState,
    clear_result: Result<(), String>,
) -> Result<(), ApiError> {
    clear_result.map_err(|_| {
        ApiError::credential_store(
            "Could not clear the saved session from the OS credential store. You remain signed in.",
        )
    })?;
    state.attempt = None;
    state.session = None;
    state.bootstrapping = false;
    state.last_error = None;
    Ok(())
}

fn snapshot_from_state(state: &AuthState) -> AuthSnapshot {
    if state.bootstrapping {
        return AuthSnapshot {
            phase: "bootstrapping",
            profile: None,
            message: None,
        };
    }
    if let Some(session) = &state.session {
        return AuthSnapshot {
            phase: "authenticated",
            profile: Some(session.profile.clone()),
            message: None,
        };
    }
    if state.attempt.is_some() {
        return AuthSnapshot {
            phase: "authorizing",
            profile: None,
            message: None,
        };
    }
    if let Some(message) = &state.last_error {
        return AuthSnapshot {
            phase: "error",
            profile: None,
            message: Some(message.clone()),
        };
    }
    AuthSnapshot {
        phase: "signedOut",
        profile: None,
        message: None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeTokenResponse {
    access_token: String,
    refresh_token: String,
}

impl NativeTokenResponse {
    fn validate(self) -> Result<TokenPair, ApiError> {
        validate_token_pair(self.access_token, self.refresh_token)
    }
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    scope: Option<String>,
}

impl OAuthTokenResponse {
    fn validate(self) -> Result<TokenPair, ApiError> {
        let refresh_token = self.refresh_token.ok_or_else(ApiError::invalid_response)?;
        if !self.token_type.eq_ignore_ascii_case("Bearer")
            || self
                .scope
                .as_deref()
                .is_some_and(|scope| !scope_set_matches(scope))
        {
            return Err(ApiError::invalid_response());
        }
        validate_token_pair(self.access_token, refresh_token)
    }
}

#[derive(Debug)]
struct TokenPair {
    access_token: String,
    refresh_token: String,
}

fn validate_token_pair(access_token: String, refresh_token: String) -> Result<TokenPair, ApiError> {
    if !valid_token(&access_token) || !valid_token(&refresh_token) {
        return Err(ApiError::invalid_response());
    }
    Ok(TokenPair {
        access_token,
        refresh_token,
    })
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= MAX_TOKEN_LENGTH
        && !value.chars().any(char::is_control)
}

fn scope_set_matches(value: &str) -> bool {
    let mut actual = BTreeSet::new();
    for token in value.split(' ') {
        if token.is_empty()
            || !token.bytes().all(|byte| {
                byte == 0x21 || (0x23..=0x5b).contains(&byte) || (0x5d..=0x7e).contains(&byte)
            })
            || !actual.insert(token)
        {
            return false;
        }
    }
    actual == SCOPE.split(' ').collect::<BTreeSet<_>>()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuthErrorResponse {
    error: String,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MethodsResponse {
    email_password_allowed: bool,
    allowed_oauth_providers: Vec<String>,
    #[serde(default)]
    oauth_client_ids: BTreeMap<String, String>,
    country_known: bool,
    #[serde(default)]
    allowed_email_domains: Vec<String>,
    #[allow(dead_code)]
    country_code: String,
}

impl MethodsResponse {
    fn validate(self) -> Result<AuthMethods, ApiError> {
        let mut seen = BTreeSet::new();
        if self.allowed_oauth_providers.iter().any(|provider| {
            !matches!(provider.as_str(), "google" | "github" | "yandex")
                || !seen.insert(provider.as_str())
        }) || self.oauth_client_ids.iter().any(|(provider, client_id)| {
            !matches!(provider.as_str(), "google" | "github" | "yandex")
                || client_id.trim().is_empty()
                || client_id.len() > 512
        }) || self.allowed_email_domains.iter().any(|domain| {
            domain.trim().is_empty() || domain.len() > 255 || domain.chars().any(char::is_control)
        }) {
            return Err(ApiError::invalid_response());
        }
        Ok(AuthMethods {
            email_password_allowed: self.email_password_allowed,
            allowed_oauth_providers: self.allowed_oauth_providers,
            oauth_client_ids: self.oauth_client_ids,
            country_known: self.country_known,
            allowed_email_domains: self.allowed_email_domains,
        })
    }
}

#[derive(Debug, Deserialize)]
struct LegalDocumentResponse {
    document_slug: String,
    title: String,
    version: String,
    language: String,
    required_for_registration: bool,
}

impl LegalDocumentResponse {
    fn validate(self) -> Result<LegalDocument, ApiError> {
        if !valid_slug(&self.document_slug)
            || self.title.trim().is_empty()
            || self.title.len() > 512
            || self.version.trim().is_empty()
            || self.version.len() > 64
            || !matches!(self.language.as_str(), "en" | "ru")
        {
            return Err(ApiError::invalid_response());
        }
        Ok(LegalDocument {
            document_slug: self.document_slug,
            title: self.title,
            version: self.version,
            language: self.language,
            required_for_registration: self.required_for_registration,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiErrorEnvelope {
    error: ApiErrorPayload,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiErrorPayload {
    code: String,
    message: String,
    params: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    fields: Vec<ApiFieldErrorWire>,
    details: Option<Value>,
    request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiFieldErrorWire {
    #[serde(default)]
    path: Vec<Value>,
    code: String,
    message: Option<String>,
    params: Option<BTreeMap<String, Value>>,
}

fn parse_api_error(status: StatusCode, bytes: &[u8]) -> ApiError {
    if let Ok(envelope) = serde_json::from_slice::<ApiErrorEnvelope>(bytes) {
        let payload = envelope.error;
        if valid_error_code(&payload.code)
            && valid_error_text(&payload.message, MAX_ERROR_MESSAGE_LENGTH)
            && payload.fields.len() <= MAX_ERROR_FIELDS
            && payload.request_id.as_deref().is_none_or(valid_request_id)
            && payload.details.as_ref().is_none_or(valid_error_details)
        {
            let params = normalize_error_params(payload.params);
            let fields = payload
                .fields
                .into_iter()
                .map(normalize_api_field_error)
                .collect::<Option<Vec<_>>>();
            let (Some(params), Some(fields)) = (params, fields) else {
                return ApiError::invalid_response();
            };
            return ApiError {
                status: status.as_u16(),
                code: payload.code,
                message: payload.message,
                fields,
                params,
                request_id: payload.request_id,
            };
        }
        return ApiError::invalid_response();
    }
    ApiError::new(
        status.as_u16(),
        "http_error",
        "Orcestr rejected the authentication request.",
    )
}

fn normalize_api_field_error(field: ApiFieldErrorWire) -> Option<ApiFieldError> {
    if field.path.len() > MAX_ERROR_PATH_PARTS
        || !valid_error_code(&field.code)
        || field
            .message
            .as_deref()
            .is_some_and(|message| !valid_error_text(message, MAX_ERROR_MESSAGE_LENGTH))
    {
        return None;
    }
    let path = field
        .path
        .into_iter()
        .map(|part| match part {
            Value::String(value)
                if !value.is_empty()
                    && value.len() <= MAX_ERROR_PATH_STRING_LENGTH
                    && !value.chars().any(char::is_control) =>
            {
                Some(ApiFieldPathPart::String(value))
            }
            Value::Number(value) => value.as_i64().and_then(|number| {
                (number.unsigned_abs() <= MAX_SAFE_JS_INTEGER as u64)
                    .then_some(ApiFieldPathPart::Number(number))
            }),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let params = normalize_error_params(field.params)?;
    Some(ApiFieldError {
        path,
        code: field.code,
        message: field.message,
        params,
    })
}

fn normalize_error_params(
    params: Option<BTreeMap<String, Value>>,
) -> Option<Option<BTreeMap<String, ApiParamValue>>> {
    let Some(params) = params else {
        return Some(None);
    };
    if params.len() > MAX_ERROR_PARAMS {
        return None;
    }
    params
        .into_iter()
        .map(|(key, value)| {
            if key.is_empty()
                || key.len() > MAX_ERROR_PARAM_KEY_LENGTH
                || key.chars().any(char::is_control)
            {
                return None;
            }
            let value = match value {
                Value::String(value)
                    if value.len() <= MAX_ERROR_PARAM_STRING_LENGTH
                        && !value.chars().any(char::is_control) =>
                {
                    ApiParamValue::String(value)
                }
                Value::Number(value) if value.is_i64() => {
                    let value = value.as_i64()?;
                    if value.unsigned_abs() > MAX_SAFE_JS_INTEGER as u64 {
                        return None;
                    }
                    ApiParamValue::Integer(value)
                }
                Value::Number(value) if value.is_u64() => {
                    let value = value.as_u64()?;
                    if value > MAX_SAFE_JS_INTEGER as u64 {
                        return None;
                    }
                    ApiParamValue::Integer(value as i64)
                }
                Value::Number(value) => {
                    let value = value.as_f64()?;
                    if !value.is_finite() || value.abs() > MAX_SAFE_JS_INTEGER as f64 {
                        return None;
                    }
                    ApiParamValue::Float(value)
                }
                Value::Bool(value) => ApiParamValue::Boolean(value),
                Value::Null => ApiParamValue::Null,
                _ => return None,
            };
            Some((key, value))
        })
        .collect::<Option<BTreeMap<_, _>>>()
        .map(Some)
}

fn valid_error_details(value: &Value) -> bool {
    if !value.is_object() {
        return false;
    }
    let mut nodes = 0;
    valid_bounded_json(value, 0, &mut nodes)
}

fn valid_bounded_json(value: &Value, depth: usize, nodes: &mut usize) -> bool {
    *nodes += 1;
    if *nodes > MAX_ERROR_DETAILS_NODES || depth > MAX_ERROR_DETAILS_DEPTH {
        return false;
    }
    match value {
        Value::Null | Value::Bool(_) => true,
        Value::String(value) => {
            value.len() <= MAX_ERROR_PARAM_STRING_LENGTH && !value.chars().any(char::is_control)
        }
        Value::Number(value) => {
            value
                .as_i64()
                .is_some_and(|number| number.unsigned_abs() <= MAX_SAFE_JS_INTEGER as u64)
                || value
                    .as_u64()
                    .is_some_and(|number| number <= MAX_SAFE_JS_INTEGER as u64)
                || value.as_f64().is_some_and(|number| {
                    number.is_finite() && number.abs() <= MAX_SAFE_JS_INTEGER as f64
                })
        }
        Value::Array(values) => {
            values.len() <= MAX_ERROR_PARAMS
                && values
                    .iter()
                    .all(|value| valid_bounded_json(value, depth + 1, nodes))
        }
        Value::Object(values) => {
            values.len() <= MAX_ERROR_PARAMS
                && values.iter().all(|(key, value)| {
                    !key.is_empty()
                        && key.len() <= MAX_ERROR_PARAM_KEY_LENGTH
                        && !key.chars().any(char::is_control)
                        && valid_bounded_json(value, depth + 1, nodes)
                })
        }
    }
}

fn valid_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ERROR_CODE_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

fn valid_error_text(value: &str, max_length: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_length
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ERROR_REQUEST_ID_LENGTH
        && !value.chars().any(char::is_control)
}

async fn bounded_body(mut response: Response) -> Result<Vec<u8>, ApiError> {
    if response.content_length().unwrap_or(0) > MAX_RESPONSE_LENGTH {
        return Err(ApiError::invalid_response());
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(MAX_RESPONSE_LENGTH) as usize,
    );
    while let Some(chunk) = response.chunk().await.map_err(|_| ApiError::network())? {
        append_bounded_chunk(&mut bytes, &chunk)?;
    }
    Ok(bytes)
}

fn append_bounded_chunk(bytes: &mut Vec<u8>, chunk: &[u8]) -> Result<(), ApiError> {
    if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_LENGTH as usize {
        return Err(ApiError::invalid_response());
    }
    bytes.extend_from_slice(chunk);
    Ok(())
}

async fn persist_then_fetch_profile<Store, StoreFuture, Fetch, FetchFuture>(
    store: Store,
    fetch: Fetch,
) -> Result<Value, ApiError>
where
    Store: FnOnce() -> StoreFuture,
    StoreFuture: std::future::Future<Output = Result<(), ApiError>>,
    Fetch: FnOnce() -> FetchFuture,
    FetchFuture: std::future::Future<Output = Result<Value, ApiError>>,
{
    store().await?;
    fetch().await
}

fn normalize_profile(kind: StoredSessionKind, profile: Value) -> Result<Value, ApiError> {
    let mut object = profile
        .as_object()
        .cloned()
        .ok_or_else(ApiError::invalid_response)?;
    match kind {
        StoredSessionKind::Password => {
            let id_valid = object.get("id").is_some_and(|id| {
                id.is_number() || id.as_str().is_some_and(|value| !value.is_empty())
            });
            let username_valid = object
                .get("username")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            if !id_valid || !username_valid {
                return Err(ApiError::invalid_response());
            }
        }
        StoredSessionKind::OAuth2 => {
            let subject = object
                .get("sub")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(ApiError::invalid_response)?
                .to_string();
            let username = ["preferred_username", "email"]
                .into_iter()
                .filter_map(|key| object.get(key).and_then(Value::as_str))
                .find(|value| !value.trim().is_empty())
                .unwrap_or(&subject)
                .to_string();
            object.insert("id".to_string(), Value::String(subject));
            object.insert("username".to_string(), Value::String(username));
        }
    }
    Ok(Value::Object(object))
}

fn validate_login(request: &LoginRequest) -> Result<(), ApiError> {
    if request.username.trim().is_empty()
        || request.username.len() > 255
        || request.username.chars().any(char::is_control)
    {
        return Err(ApiError::command("Email or username is invalid."));
    }
    if request.password.is_empty() || request.password.len() > 1_024 {
        return Err(ApiError::command("Password is invalid."));
    }
    if request.accepted_legal_documents.len() > MAX_LEGAL_DOCUMENTS
        || request.accepted_legal_documents.iter().any(|document| {
            !valid_slug(&document.document_slug)
                || document.version.trim().is_empty()
                || document.version.len() > 64
                || !matches!(document.language.as_str(), "en" | "ru")
        })
    {
        return Err(ApiError::command("Legal consent payload is invalid."));
    }
    Ok(())
}

fn validate_email(email: &str) -> Result<(), ApiError> {
    if !(3..=255).contains(&email.len())
        || email.trim() != email
        || !email.contains('@')
        || email.chars().any(char::is_control)
    {
        return Err(ApiError::command("Email is invalid."));
    }
    Ok(())
}

fn validate_password_reset_confirm(request: &PasswordResetConfirmRequest) -> Result<(), ApiError> {
    validate_email(&request.email)?;
    if !(4..=32).contains(&request.code.len()) || request.code.chars().any(char::is_control) {
        return Err(ApiError::command("Password reset code is invalid."));
    }
    if !(8..=1_024).contains(&request.password.len()) {
        return Err(ApiError::command("New password length is invalid."));
    }
    Ok(())
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_legal_document_url(value: &str) -> Result<Url, ApiError> {
    if value.len() > 2_048 {
        return Err(ApiError::command("Legal document URL is too long."));
    }
    let url = Url::parse(value).map_err(|_| ApiError::command("Legal document URL is invalid."))?;
    if url.scheme() != "https"
        || url.host_str() != Some("orcestr.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::command("Legal document URL is not allowed."));
    }
    let segments = url
        .path_segments()
        .ok_or_else(|| ApiError::command("Legal document path is invalid."))?
        .collect::<Vec<_>>();
    let slug = match segments.as_slice() {
        ["legal", slug] | ["ru", "legal", slug] => *slug,
        _ => return Err(ApiError::command("Legal document path is not allowed.")),
    };
    if !valid_slug(slug) {
        return Err(ApiError::command("Legal document slug is invalid."));
    }
    Ok(url)
}

fn build_oauth_browser_login_url(
    config: &AuthConfig,
    provider: OAuthProvider,
    state: &str,
    code_challenge: &str,
) -> Result<Url, ApiError> {
    validate_state(state)?;
    if code_challenge.len() != 43
        || !code_challenge
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApiError::command("OAuth code challenge is invalid."));
    }

    let mut browser_login_url = config.browser_login_endpoint().map_err(|_| {
        ApiError::new(
            500,
            "invalid_auth_configuration",
            "The Orcestr browser authorization endpoint is invalid.",
        )
    })?;
    let mut authorization_url = config.authorization_endpoint.clone();
    authorization_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    authorization_url.set_fragment(Some(&format!("provider={}", provider.as_str())));

    let mut next = authorization_url.path().to_string();
    if let Some(query) = authorization_url.query() {
        next.push('?');
        next.push_str(query);
    }
    if let Some(fragment) = authorization_url.fragment() {
        next.push('#');
        next.push_str(fragment);
    }
    if authorization_url.as_str().len() > MAX_AUTHORIZATION_URL_LENGTH
        || next.len() > MAX_AUTHORIZATION_URL_LENGTH
    {
        return Err(ApiError::command("Authorization URL is too long."));
    }

    browser_login_url
        .query_pairs_mut()
        .append_pair("next", &next)
        .append_pair("provider", provider.as_str());
    validate_built_browser_login_url(&browser_login_url, &authorization_url, &next, provider)?;
    Ok(browser_login_url)
}

fn validate_built_browser_login_url(
    browser_login_url: &Url,
    authorization_url: &Url,
    next: &str,
    provider: OAuthProvider,
) -> Result<(), ApiError> {
    let provider_fragment = format!("provider={}", provider.as_str());
    let same_origin = browser_login_url.scheme() == authorization_url.scheme()
        && browser_login_url.host_str() == authorization_url.host_str()
        && browser_login_url.port() == authorization_url.port();
    if !same_origin
        || browser_login_url.path() != "/login"
        || browser_login_url.fragment().is_some()
        || browser_login_url.as_str().len() > MAX_BROWSER_LOGIN_URL_LENGTH
        || authorization_url.path() != "/oauth/authorize"
        || authorization_url.fragment() != Some(provider_fragment.as_str())
        || !next.starts_with("/oauth/authorize?")
        || !next.ends_with(&format!("#{provider_fragment}"))
    {
        return Err(ApiError::new(
            500,
            "invalid_auth_configuration",
            "The Orcestr browser authorization URL could not be built safely.",
        ));
    }

    let query = browser_login_url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if query
        != [
            ("next".to_string(), next.to_string()),
            ("provider".to_string(), provider.as_str().to_string()),
        ]
    {
        return Err(ApiError::new(
            500,
            "invalid_auth_configuration",
            "The Orcestr browser login parameters are invalid.",
        ));
    }

    let resolved = browser_login_url.join(next).map_err(|_| {
        ApiError::new(
            500,
            "invalid_auth_configuration",
            "The Orcestr authorization return path is invalid.",
        )
    })?;
    if resolved != *authorization_url {
        return Err(ApiError::new(
            500,
            "invalid_auth_configuration",
            "The Orcestr authorization return path changed origin.",
        ));
    }
    Ok(())
}

fn random_base64_url(byte_length: usize) -> Result<String, ApiError> {
    let mut bytes = vec![0_u8; byte_length];
    getrandom::fill(&mut bytes).map_err(|_| {
        ApiError::new(
            500,
            "secure_random_unavailable",
            "Secure authorization could not be initialized.",
        )
    })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[derive(Debug)]
enum ParsedCallback {
    Success { code: String, state: String },
    Denied { state: String },
}

impl ParsedCallback {
    fn state(&self) -> &str {
        match self {
            Self::Success { state, .. } | Self::Denied { state } => state,
        }
    }
}

fn parse_callback(value: &str) -> Result<ParsedCallback, ApiError> {
    if value.len() > MAX_AUTHORIZATION_URL_LENGTH {
        return Err(ApiError::command("OAuth callback is too long."));
    }
    let parsed = Url::parse(value).map_err(|_| ApiError::command("OAuth callback is invalid."))?;
    if parsed.fragment().is_some() {
        return Err(ApiError::command(
            "OAuth callback must not contain a fragment.",
        ));
    }
    let mut base = parsed.clone();
    base.set_query(None);
    let expected = Url::parse(REDIRECT_URI).expect("registered callback URI must be valid");
    if base != expected {
        return Err(ApiError::command("OAuth callback URI is not registered."));
    }

    let query = strict_query(&parsed, &["code", "state", "error", "error_description"])?;
    let state = query
        .get("state")
        .ok_or_else(|| ApiError::command("OAuth callback state is missing."))?
        .clone();
    validate_state(&state)?;
    match (query.get("code"), query.get("error")) {
        (Some(code), None)
            if !code.is_empty() && code.len() <= MAX_AUTH_CODE_LENGTH && query.len() == 2 =>
        {
            Ok(ParsedCallback::Success {
                code: code.clone(),
                state,
            })
        }
        (None, Some(error)) if !error.is_empty() && query.len() <= 3 => {
            Ok(ParsedCallback::Denied { state })
        }
        _ => Err(ApiError::command("OAuth callback parameters are invalid.")),
    }
}

fn validate_state(value: &str) -> Result<(), ApiError> {
    if !(16..=512).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
    {
        return Err(ApiError::command("OAuth state is invalid."));
    }
    Ok(())
}

fn strict_query(url: &Url, allowed: &[&str]) -> Result<BTreeMap<String, String>, ApiError> {
    let mut result = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if !allowed.contains(&key.as_ref()) {
            return Err(ApiError::command(
                "OAuth callback contains an unexpected parameter.",
            ));
        }
        if result
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(ApiError::command(
                "OAuth callback contains a duplicate parameter.",
            ));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_requires_exact_registered_uri_and_contract() {
        let state = "state_0123456789abcdef0123456789";
        let valid = format!("{REDIRECT_URI}?code=one-time-code&state={state}");
        assert!(matches!(
            parse_callback(&valid).unwrap(),
            ParsedCallback::Success { .. }
        ));
        let wrong_host =
            format!("com.orcestr.realtranslate://attacker/callback?code=x&state={state}");
        assert!(parse_callback(&wrong_host).is_err());
        assert!(parse_callback(&format!("{valid}&state={state}")).is_err());
        assert!(parse_callback(&format!("{valid}&extra=value")).is_err());
    }

    #[test]
    fn browser_login_url_wraps_exact_public_client_authorization_target() {
        let config = AuthConfig {
            authorization_endpoint: Url::parse("https://orcestr.com/oauth/authorize").unwrap(),
            api_base: Url::parse("https://orcestr.com/api/v1/").unwrap(),
        };
        let state = "state_0123456789abcdef0123456789";
        let challenge = "A".repeat(43);
        let browser =
            build_oauth_browser_login_url(&config, OAuthProvider::Google, state, &challenge)
                .unwrap();

        assert_eq!(
            browser.origin().ascii_serialization(),
            "https://orcestr.com"
        );
        assert_eq!(browser.path(), "/login");
        assert!(browser.fragment().is_none());
        assert!(browser.as_str().contains("next=%2Foauth%2Fauthorize%3F"));
        let parameters = browser.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            parameters.get("provider").map(|value| value.as_ref()),
            Some("google")
        );
        let next = parameters.get("next").unwrap();
        assert!(next.starts_with("/oauth/authorize?response_type=code&"));
        assert!(next.ends_with("#provider=google"));

        let authorization = browser.join(next).unwrap();
        assert_eq!(authorization.path(), "/oauth/authorize");
        assert_eq!(authorization.fragment(), Some("provider=google"));
        let query = authorization.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            query.get("response_type").map(|value| value.as_ref()),
            Some("code")
        );
        assert_eq!(
            query.get("client_id").map(|value| value.as_ref()),
            Some(CLIENT_ID)
        );
        assert_eq!(
            query.get("redirect_uri").map(|value| value.as_ref()),
            Some(REDIRECT_URI)
        );
        assert_eq!(query.get("scope").map(|value| value.as_ref()), Some(SCOPE));
        assert_eq!(query.get("state").map(|value| value.as_ref()), Some(state));
        assert_eq!(
            query.get("code_challenge").map(|value| value.as_ref()),
            Some(challenge.as_str())
        );
        assert_eq!(
            query
                .get("code_challenge_method")
                .map(|value| value.as_ref()),
            Some("S256")
        );
    }

    #[test]
    fn browser_login_builder_rejects_untrusted_config_and_unbounded_values() {
        let config = |authorization_endpoint: &str| AuthConfig {
            authorization_endpoint: Url::parse(authorization_endpoint).unwrap(),
            api_base: Url::parse("https://orcestr.com/api/v1/").unwrap(),
        };
        let state = "state_0123456789abcdef0123456789";
        let challenge = "A".repeat(43);

        for endpoint in [
            "https://attacker.test/oauth/authorize",
            "https://orcestr.com.attacker.test/oauth/authorize",
            "https://orcestr.com/oauth/authorize/",
            "https://orcestr.com/oauth/authorize?next=/admin",
        ] {
            assert!(build_oauth_browser_login_url(
                &config(endpoint),
                OAuthProvider::Github,
                state,
                &challenge,
            )
            .is_err());
        }
        assert!(build_oauth_browser_login_url(
            &config("https://orcestr.com/oauth/authorize"),
            OAuthProvider::Github,
            "short",
            &challenge,
        )
        .is_err());
        assert!(build_oauth_browser_login_url(
            &config("https://orcestr.com/oauth/authorize"),
            OAuthProvider::Github,
            state,
            &"A".repeat(44),
        )
        .is_err());

        let mut authorization = Url::parse("https://orcestr.com/oauth/authorize").unwrap();
        authorization.set_query(Some(&format!(
            "state={}",
            "x".repeat(MAX_AUTHORIZATION_URL_LENGTH)
        )));
        authorization.set_fragment(Some("provider=github"));
        let mut browser = Url::parse("https://orcestr.com/login").unwrap();
        let next = format!(
            "/oauth/authorize?{}#provider=github",
            authorization.query().unwrap()
        );
        browser
            .query_pairs_mut()
            .append_pair("next", &next)
            .append_pair("provider", "github");
        assert!(validate_built_browser_login_url(
            &browser,
            &authorization,
            &next,
            OAuthProvider::Github,
        )
        .is_err());
    }

    #[test]
    fn access_token_is_never_part_of_renderer_snapshot() {
        let state = AuthState {
            session: Some(Session {
                access_token: "secret-access-token".to_string(),
                kind: StoredSessionKind::Password,
                profile: json!({"id": 1, "username": "user", "email": "user@example.test"}),
            }),
            ..AuthState::default()
        };
        let serialized = serde_json::to_string(&snapshot_from_state(&state)).unwrap();
        assert!(!serialized.contains("secret-access-token"));
        assert!(serialized.contains("user@example.test"));
    }

    #[test]
    fn native_and_oauth_profiles_are_normalized_to_auth_user() {
        let native = normalize_profile(
            StoredSessionKind::Password,
            json!({"id": 7, "username": "native", "email": "n@example.test"}),
        )
        .unwrap();
        assert_eq!(native["username"], "native");

        let oauth = normalize_profile(
            StoredSessionKind::OAuth2,
            json!({"sub": "42", "preferred_username": "oauth-user"}),
        )
        .unwrap();
        assert_eq!(oauth["id"], "42");
        assert_eq!(oauth["username"], "oauth-user");
    }

    #[test]
    fn token_contracts_are_strict_and_oauth_scope_is_exact() {
        assert!(NativeTokenResponse {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
        }
        .validate()
        .is_ok());
        assert!(NativeTokenResponse {
            access_token: "".to_string(),
            refresh_token: "refresh".to_string(),
        }
        .validate()
        .is_err());

        let response = |scope: Option<&str>| OAuthTokenResponse {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            scope: scope.map(str::to_string),
        };
        assert!(response(Some("email offline_access openid profile"))
            .validate()
            .is_ok());
        assert!(response(None).validate().is_ok());
        assert!(response(Some("openid profile email")).validate().is_err());
        assert!(response(Some("openid profile email offline_access admin"))
            .validate()
            .is_err());
    }

    #[test]
    fn structured_backend_errors_remain_localizable() {
        let bytes = br#"{"error":{"code":"invalid_credentials","message":"Invalid credentials","params":{"attempts":2},"fields":[{"path":["password"],"code":"invalid","params":{"min_length":8}}],"request_id":"req-1"}}"#;
        let error = parse_api_error(StatusCode::UNAUTHORIZED, bytes);
        assert_eq!(error.status, 401);
        assert_eq!(error.code, "invalid_credentials");
        assert_eq!(error.fields.len(), 1);
        assert_eq!(
            serde_json::to_value(&error.params).unwrap(),
            json!({"attempts": 2})
        );
        assert_eq!(
            serde_json::to_value(&error.fields).unwrap()[0]["params"],
            json!({"min_length": 8})
        );
        assert_eq!(error.request_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn malformed_structured_error_fields_never_cross_ipc() {
        for bytes in [
            br#"{"error":{"code":"invalid_credentials","message":"Invalid","fields":[{"path":[{"secret":true}],"code":"invalid"}]}}"#.as_slice(),
            br#"{"error":{"code":"invalid_credentials","message":"Invalid","fields":[{"path":[1.5],"code":"invalid"}]}}"#.as_slice(),
            br#"{"error":{"code":"invalid credentials","message":"Invalid","fields":[]}}"#.as_slice(),
            br#"{"error":{"code":"invalid_credentials","message":"Invalid","fields":[],"request_id":"\u0000"}}"#.as_slice(),
        ] {
            let error = parse_api_error(StatusCode::BAD_REQUEST, bytes);
            assert_eq!(error.status, 502);
            assert_eq!(error.code, "invalid_error_response");
            assert!(error.fields.is_empty());
        }

        let numeric = br#"{"error":{"code":"invalid_value","message":"Invalid","fields":[{"path":["items",2],"code":"invalid"}]}}"#;
        let error = parse_api_error(StatusCode::UNPROCESSABLE_ENTITY, numeric);
        assert_eq!(
            serde_json::to_value(&error.fields).unwrap()[0]["path"],
            json!(["items", 2])
        );
    }

    #[test]
    fn auth_methods_serialize_exact_shared_snake_case_contract() {
        let methods = AuthMethods {
            email_password_allowed: true,
            allowed_oauth_providers: vec!["google".to_string()],
            oauth_client_ids: BTreeMap::from([("google".to_string(), "google-client".to_string())]),
            country_known: true,
            allowed_email_domains: vec![],
        };
        assert_eq!(
            serde_json::to_value(methods).unwrap(),
            json!({
                "email_password_allowed": true,
                "allowed_oauth_providers": ["google"],
                "oauth_client_ids": {"google": "google-client"},
                "country_known": true,
                "allowed_email_domains": [],
            })
        );
    }

    #[test]
    fn legal_and_login_payloads_reject_unbounded_input() {
        assert!(valid_slug("privacy-policy"));
        assert!(!valid_slug("../privacy"));
        assert!(validate_login(&LoginRequest {
            username: "user@example.test".to_string(),
            password: "password".to_string(),
            accepted_legal_documents: vec![LegalAcceptance {
                document_slug: "privacy-policy".to_string(),
                version: "1.0".to_string(),
                language: "en".to_string(),
            }],
        })
        .is_ok());
        assert!(validate_login(&LoginRequest {
            username: "u".repeat(255),
            password: "p".repeat(1_024),
            accepted_legal_documents: vec![],
        })
        .is_ok());
        assert!(validate_login(&LoginRequest {
            username: "u".repeat(256),
            password: "password".to_string(),
            accepted_legal_documents: vec![],
        })
        .is_err());

        assert!(
            validate_password_reset_confirm(&PasswordResetConfirmRequest {
                email: "user@example.test".to_string(),
                code: "c".repeat(32),
                password: "p".repeat(1_024),
            })
            .is_ok()
        );
        assert!(
            validate_password_reset_confirm(&PasswordResetConfirmRequest {
                email: "user@example.test".to_string(),
                code: "c".repeat(33),
                password: "p".repeat(1_024),
            })
            .is_err()
        );
        assert!(
            validate_password_reset_confirm(&PasswordResetConfirmRequest {
                email: "user@example.test".to_string(),
                code: "123456".to_string(),
                password: "p".repeat(1_025),
            })
            .is_err()
        );
    }

    #[test]
    fn response_body_limit_is_cumulative_without_content_length() {
        let mut bytes = vec![0; MAX_RESPONSE_LENGTH as usize - 1];
        append_bounded_chunk(&mut bytes, &[1]).unwrap();
        assert_eq!(bytes.len(), MAX_RESPONSE_LENGTH as usize);
        let error = append_bounded_chunk(&mut bytes, &[2]).unwrap_err();
        assert_eq!(error.code, "invalid_error_response");
        assert_eq!(bytes.len(), MAX_RESPONSE_LENGTH as usize);
    }

    #[test]
    fn rotated_refresh_is_persisted_before_profile_network_work() {
        let steps = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let store_steps = steps.clone();
        let fetch_steps = steps.clone();
        let result = tauri::async_runtime::block_on(persist_then_fetch_profile(
            move || async move {
                store_steps.lock().unwrap().push("store");
                Ok(())
            },
            move || async move {
                assert_eq!(fetch_steps.lock().unwrap().as_slice(), &["store"]);
                fetch_steps.lock().unwrap().push("fetch");
                Err(ApiError::network())
            },
        ));
        assert!(result.is_err());
        assert_eq!(steps.lock().unwrap().as_slice(), &["store", "fetch"]);
    }

    #[test]
    fn logout_keeps_memory_session_until_keyring_clear_succeeds() {
        let session = Session {
            access_token: "memory-access".to_string(),
            kind: StoredSessionKind::Password,
            profile: json!({"id": 42, "username": "user"}),
        };
        let mut state = AuthState {
            session: Some(session),
            ..AuthState::default()
        };

        let error =
            apply_logout_clear_result(&mut state, Err("credential store unavailable".to_string()))
                .unwrap_err();
        assert_eq!(error.code, "credential_store_unavailable");
        assert_eq!(
            state.session.as_ref().unwrap().access_token,
            "memory-access"
        );
        assert_eq!(snapshot_from_state(&state).phase, "authenticated");

        apply_logout_clear_result(&mut state, Ok(())).unwrap();
        assert!(state.session.is_none());
        assert_eq!(snapshot_from_state(&state).phase, "signedOut");
    }

    #[test]
    fn oauth_cancel_and_retry_do_not_silently_replace_active_state() {
        let mut state = AuthState {
            attempt: Some(AuthAttempt {
                state: "active-state".to_string(),
                code_verifier: "verifier".to_string(),
                created_at: Instant::now(),
            }),
            ..AuthState::default()
        };

        let error = ensure_oauth_start_allowed(&state).unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(error.code, "oauth_authorization_in_progress");

        clear_oauth_attempt(&mut state);
        assert!(state.attempt.is_none());
        assert!(ensure_oauth_start_allowed(&state).is_ok());
        assert_eq!(snapshot_from_state(&state).phase, "signedOut");
    }

    #[test]
    fn current_user_refetch_preserves_active_oauth_attempt() {
        let mut state = AuthState {
            attempt: Some(AuthAttempt {
                state: "expected-state".to_string(),
                code_verifier: "verifier".to_string(),
                created_at: Instant::now(),
            }),
            ..AuthState::default()
        };

        let error = session_for_me(&state).unwrap_err();
        assert_eq!(error.code, "not_authenticated");
        assert!(state.attempt.is_some());
        assert!(matches!(
            take_callback_attempt(&mut state, "expected-state"),
            CallbackAttempt::Active(_)
        ));
    }

    #[test]
    fn matching_expired_callback_errors_but_mismatch_is_silent() {
        let expired_at = Instant::now() - ATTEMPT_LIFETIME - Duration::from_secs(1);
        let attempt = || AuthAttempt {
            state: "expected-state".to_string(),
            code_verifier: "verifier".to_string(),
            created_at: expired_at,
        };
        let mut state = AuthState {
            attempt: Some(attempt()),
            ..AuthState::default()
        };

        assert!(matches!(
            take_callback_attempt(&mut state, "different-state"),
            CallbackAttempt::Ignored
        ));
        assert!(state.attempt.is_some());
        assert!(state.last_error.is_none());

        assert!(matches!(
            take_callback_attempt(&mut state, "expected-state"),
            CallbackAttempt::Expired
        ));
        assert!(state.attempt.is_none());
        assert_eq!(
            state.last_error.as_deref(),
            Some("Authorization session expired. Try again.")
        );
        assert_eq!(snapshot_from_state(&state).phase, "error");
    }

    #[test]
    fn legal_document_opener_is_exactly_allowlisted() {
        for value in [
            "https://orcestr.com/legal/privacy-policy",
            "https://orcestr.com/ru/legal/user-agreement",
        ] {
            assert!(validate_legal_document_url(value).is_ok());
        }
        for value in [
            "http://orcestr.com/legal/privacy-policy",
            "https://www.orcestr.com/legal/privacy-policy",
            "https://orcestr.com.attacker.test/legal/privacy-policy",
            "https://orcestr.com/legal/../admin",
            "https://orcestr.com/legal/privacy-policy?next=https://attacker.test",
            "https://orcestr.com/ru/legal/privacy-policy/extra",
        ] {
            assert!(validate_legal_document_url(value).is_err(), "{value}");
        }
    }
}
