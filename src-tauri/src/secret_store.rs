use serde::{Deserialize, Serialize};
use std::{future::Future, time::Duration};

const KEYRING_SERVICE: &str = "com.orcestr.realtranslate";
const SESSION_ACCOUNT: &str = "auth.refresh-session";
const OPENAI_ACCOUNT: &str = "provider.openai.api-key";
const SESSION_VERSION: u8 = 1;
const MAX_REFRESH_TOKEN_LENGTH: usize = 24 * 1024;
const MAX_OPENAI_API_KEY_LENGTH: usize = 8 * 1024;
const CREDENTIAL_READ_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiKeyStatus {
    pub configured: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredSessionKind {
    Password,
    OAuth2,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredSession {
    version: u8,
    pub kind: StoredSessionKind,
    pub refresh_token: String,
}

impl StoredSession {
    pub fn new(kind: StoredSessionKind, refresh_token: String) -> Result<Self, String> {
        validate_refresh_token(&refresh_token)?;
        Ok(Self {
            version: SESSION_VERSION,
            kind,
            refresh_token,
        })
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != SESSION_VERSION {
            return Err("Stored session version is invalid".to_string());
        }
        validate_refresh_token(&self.refresh_token)
    }
}

#[derive(Debug, Default)]
pub struct SecretStore;

impl SecretStore {
    pub fn new() -> Self {
        Self
    }

    pub async fn get_session(&self) -> Result<Option<StoredSession>, String> {
        let read = tauri::async_runtime::spawn_blocking(|| {
            let entry = keyring::Entry::new(KEYRING_SERVICE, SESSION_ACCOUNT)
                .map_err(|error| format!("OS credential store is unavailable: {error}"))?;
            match entry.get_password() {
                Ok(value) => {
                    let session: StoredSession = serde_json::from_str(&value)
                        .map_err(|_| "Stored session is invalid".to_string())?;
                    session.validate()?;
                    Ok(Some(session))
                }
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(format!("Could not read the stored session: {error}")),
            }
        });
        bounded_credential_read(
            async move {
                read.await
                    .map_err(|error| format!("Credential task failed: {error}"))?
            },
            CREDENTIAL_READ_TIMEOUT,
        )
        .await
    }

    pub async fn set_session(&self, session: &StoredSession) -> Result<(), String> {
        session.validate()?;
        let value = serde_json::to_string(session)
            .map_err(|_| "Could not encode the stored session".to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            let entry = keyring::Entry::new(KEYRING_SERVICE, SESSION_ACCOUNT)
                .map_err(|error| format!("OS credential store is unavailable: {error}"))?;
            entry
                .set_password(&value)
                .map_err(|error| format!("Could not store the session: {error}"))
        })
        .await
        .map_err(|error| format!("Credential task failed: {error}"))?
    }

    pub async fn clear_session(&self) -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(|| {
            let entry = keyring::Entry::new(KEYRING_SERVICE, SESSION_ACCOUNT)
                .map_err(|error| format!("OS credential store is unavailable: {error}"))?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(format!("Could not clear the stored session: {error}")),
            }
        })
        .await
        .map_err(|error| format!("Credential task failed: {error}"))?
    }

    /// Returns only whether a provider credential exists. The credential itself
    /// never crosses the Tauri IPC boundary.
    pub async fn openai_key_status(&self) -> Result<OpenAiKeyStatus, String> {
        Ok(OpenAiKeyStatus {
            configured: self.get_openai_api_key().await?.is_some(),
        })
    }

    pub async fn get_openai_api_key(&self) -> Result<Option<String>, String> {
        match read_secret(OPENAI_ACCOUNT).await? {
            Some(value) => Ok(Some(normalize_openai_api_key(&value)?.to_string())),
            None => Ok(None),
        }
    }

    pub async fn set_openai_api_key(&self, value: &str) -> Result<OpenAiKeyStatus, String> {
        let value = normalize_openai_api_key(value)?;
        write_secret(OPENAI_ACCOUNT, value).await?;
        Ok(OpenAiKeyStatus { configured: true })
    }

    pub async fn clear_openai_api_key(&self) -> Result<OpenAiKeyStatus, String> {
        delete_secret(OPENAI_ACCOUNT).await?;
        Ok(OpenAiKeyStatus { configured: false })
    }
}

async fn read_secret(account: &'static str) -> Result<Option<String>, String> {
    let read = tauri::async_runtime::spawn_blocking(move || {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|error| format!("OS credential store is unavailable: {error}"))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("Could not read the stored credential: {error}")),
        }
    });
    tokio::time::timeout(CREDENTIAL_READ_TIMEOUT, async move {
        read.await
            .map_err(|error| format!("Credential task failed: {error}"))?
    })
    .await
    .map_err(|_| "Timed out while reading the OS credential store.".to_string())?
}

async fn write_secret(account: &'static str, value: &str) -> Result<(), String> {
    let value = value.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|error| format!("OS credential store is unavailable: {error}"))?;
        entry
            .set_password(&value)
            .map_err(|error| format!("Could not store the credential: {error}"))
    })
    .await
    .map_err(|error| format!("Credential task failed: {error}"))?
}

async fn delete_secret(account: &'static str) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|error| format!("OS credential store is unavailable: {error}"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("Could not clear the stored credential: {error}")),
        }
    })
    .await
    .map_err(|error| format!("Credential task failed: {error}"))?
}

fn normalize_openai_api_key(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_OPENAI_API_KEY_LENGTH
        || value.chars().any(|character| {
            character.is_control() || character.is_whitespace() || !character.is_ascii()
        })
    {
        return Err("OpenAI API key has an invalid format".to_string());
    }
    Ok(value)
}

async fn bounded_credential_read<F>(
    read: F,
    timeout: Duration,
) -> Result<Option<StoredSession>, String>
where
    F: Future<Output = Result<Option<StoredSession>, String>>,
{
    tokio::time::timeout(timeout, read).await.map_err(|_| {
        "Timed out while reading the saved session from the OS credential store.".to_string()
    })?
}

fn validate_refresh_token(token: &str) -> Result<(), String> {
    if token.trim().is_empty()
        || token.trim() != token
        || token.len() > MAX_REFRESH_TOKEN_LENGTH
        || token.chars().any(char::is_control)
    {
        return Err("Stored refresh token is invalid".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_session_is_versioned_and_strict() {
        let session = StoredSession::new(
            StoredSessionKind::Password,
            "valid-refresh-token".to_string(),
        )
        .unwrap();
        let encoded = serde_json::to_string(&session).unwrap();
        assert_eq!(
            serde_json::from_str::<StoredSession>(&encoded).unwrap(),
            session
        );
        assert!(serde_json::from_str::<StoredSession>(
            r#"{"version":1,"kind":"password","refresh_token":"x","extra":true}"#
        )
        .is_err());
    }

    #[test]
    fn refresh_token_validation_rejects_unsafe_values() {
        assert!(validate_refresh_token("").is_err());
        assert!(validate_refresh_token(" token").is_err());
        assert!(validate_refresh_token("token\n").is_err());
        assert!(validate_refresh_token("valid-refresh-token").is_ok());
        assert!(validate_refresh_token(&"x".repeat(MAX_REFRESH_TOKEN_LENGTH + 1)).is_err());
    }

    #[test]
    fn pending_credential_read_times_out() {
        let error = tauri::async_runtime::block_on(bounded_credential_read(
            std::future::pending(),
            Duration::ZERO,
        ))
        .unwrap_err();
        assert_eq!(
            error,
            "Timed out while reading the saved session from the OS credential store."
        );
    }

    #[test]
    fn openai_api_key_validation_is_bounded_and_never_prefix_specific() {
        assert_eq!(
            normalize_openai_api_key("  future-provider-key_123  ").unwrap(),
            "future-provider-key_123"
        );
        assert!(normalize_openai_api_key("").is_err());
        assert!(normalize_openai_api_key("key with spaces").is_err());
        assert!(normalize_openai_api_key("key\nsecret").is_err());
        assert!(normalize_openai_api_key(&"x".repeat(MAX_OPENAI_API_KEY_LENGTH + 1)).is_err());
    }
}
