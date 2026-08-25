use url::Url;

pub const CLIENT_ID: &str = "orcestr-real-translate";
pub const REDIRECT_URI: &str = "com.orcestr.realtranslate://oauth/callback";
pub const SCOPE: &str = "openid profile email offline_access";

const PRODUCTION_AUTHORIZE_URL: &str = "https://orcestr.com/oauth/authorize";
const PRODUCTION_API_BASE_URL: &str = "https://orcestr.com/api/v1";
const LOGIN_PATH: &str = "/login";

#[derive(Clone, Debug)]
pub struct AuthConfig {
    pub authorization_endpoint: Url,
    pub api_base: Url,
}

impl AuthConfig {
    pub fn load() -> Result<Self, String> {
        let authorization_endpoint = load_endpoint(
            "ORCESTR_AUTHORIZE_URL",
            PRODUCTION_AUTHORIZE_URL,
            EndpointKind::Authorization,
        )?;
        let mut api_base = load_endpoint(
            "ORCESTR_API_BASE_URL",
            PRODUCTION_API_BASE_URL,
            EndpointKind::ApiBase,
        )?;
        if !api_base.path().ends_with('/') {
            api_base.set_path("/api/v1/");
        }

        Ok(Self {
            authorization_endpoint,
            api_base,
        })
    }

    pub fn methods_endpoint(&self) -> Url {
        let mut url = self.api("auth/methods/");
        // Provider client IDs must match the validated browser origin that will
        // own the web SDK callback and its browser sessionStorage.
        let browser_origin = self.authorization_endpoint.origin().ascii_serialization();
        url.query_pairs_mut().append_pair("origin", &browser_origin);
        url
    }

    pub fn browser_login_endpoint(&self) -> Result<Url, &'static str> {
        let allow_localhost = cfg!(debug_assertions)
            && self.authorization_endpoint.scheme() == "http"
            && self.authorization_endpoint.host_str() == Some("localhost")
            && self.authorization_endpoint.port().is_some();
        validate_endpoint(
            &self.authorization_endpoint,
            allow_localhost,
            EndpointKind::Authorization,
        )?;
        let mut url = self.authorization_endpoint.clone();
        url.set_path(LOGIN_PATH);
        debug_assert!(url.query().is_none() && url.fragment().is_none());
        Ok(url)
    }

    pub fn native_login_endpoint(&self) -> Url {
        self.api("auth/token/login/")
    }

    pub fn native_refresh_endpoint(&self) -> Url {
        self.api("auth/token/refresh/")
    }

    pub fn native_logout_endpoint(&self) -> Url {
        self.api("auth/token/logout/")
    }

    pub fn native_me_endpoint(&self) -> Url {
        self.api("auth/me/")
    }

    pub fn password_reset_request_endpoint(&self) -> Url {
        self.api("auth/password/reset/request/")
    }

    pub fn password_reset_confirm_endpoint(&self) -> Url {
        self.api("auth/password/reset/confirm/")
    }

    pub fn legal_documents_endpoint(&self, language: &str) -> Url {
        let mut url = self.api("legal/registration-documents/");
        url.query_pairs_mut().append_pair("language", language);
        url
    }

    pub fn oauth_token_endpoint(&self) -> Url {
        self.api("auth/oauth2/token/")
    }

    pub fn oauth_revoke_endpoint(&self) -> Url {
        self.api("auth/oauth2/revoke/")
    }

    pub fn oauth_userinfo_endpoint(&self) -> Url {
        self.api("auth/oauth2/userinfo/")
    }

    fn api(&self, path: &str) -> Url {
        self.api_base
            .join(path)
            .expect("validated API base must accept an allowlisted relative path")
    }
}

#[derive(Clone, Copy)]
enum EndpointKind {
    Authorization,
    ApiBase,
}

fn load_endpoint(environment_name: &str, default: &str, kind: EndpointKind) -> Result<Url, String> {
    let explicit = std::env::var(environment_name).ok();
    let value = explicit.as_deref().unwrap_or(default);
    let parsed = Url::parse(value.trim())
        .map_err(|_| format!("{environment_name} is not a valid absolute URL"))?;
    let allow_explicit_localhost = cfg!(debug_assertions) && explicit.is_some();
    validate_endpoint(&parsed, allow_explicit_localhost, kind)
        .map_err(|message| format!("{environment_name} {message}"))?;
    Ok(parsed)
}

fn validate_endpoint(
    url: &Url,
    allow_explicit_localhost: bool,
    kind: EndpointKind,
) -> Result<(), &'static str> {
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("must not contain credentials, a query, or a fragment");
    }

    let host = url.host_str().ok_or("must contain a host")?;
    let production = url.scheme() == "https" && host == "orcestr.com" && url.port().is_none();
    let explicit_localhost = allow_explicit_localhost
        && url.scheme() == "http"
        && host == "localhost"
        && url.port().is_some();
    if !production && !explicit_localhost {
        return Err(
            "must use https://orcestr.com; debug localhost requires an explicit environment override",
        );
    }

    match kind {
        EndpointKind::Authorization if url.path() != "/oauth/authorize" => {
            Err("must use the exact /oauth/authorize path")
        }
        EndpointKind::ApiBase if !matches!(url.path(), "/api/v1" | "/api/v1/") => {
            Err("must use the exact /api/v1 path")
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_is_the_default_and_exact_origin() {
        for (value, kind) in [
            (PRODUCTION_AUTHORIZE_URL, EndpointKind::Authorization),
            (PRODUCTION_API_BASE_URL, EndpointKind::ApiBase),
        ] {
            assert!(validate_endpoint(&Url::parse(value).unwrap(), false, kind).is_ok());
        }

        for value in [
            "http://orcestr.com/api/v1",
            "https://www.orcestr.com/api/v1",
            "https://orcestr.com.attacker.test/api/v1",
            "https://orcestr.com:444/api/v1",
            "https://user@orcestr.com/api/v1",
            "https://orcestr.com/api/v2",
        ] {
            assert!(
                validate_endpoint(&Url::parse(value).unwrap(), false, EndpointKind::ApiBase)
                    .is_err()
            );
        }
    }

    #[test]
    fn localhost_is_only_an_explicit_debug_override() {
        let api = Url::parse("http://localhost:3934/api/v1").unwrap();
        assert!(validate_endpoint(&api, true, EndpointKind::ApiBase).is_ok());
        assert!(validate_endpoint(&api, false, EndpointKind::ApiBase).is_err());

        for value in [
            "http://localhost/api/v1",
            "http://127.0.0.1:3934/api/v1",
            "https://localhost:3934/api/v1",
            "http://localhost.attacker.test:3934/api/v1",
        ] {
            assert!(
                validate_endpoint(&Url::parse(value).unwrap(), true, EndpointKind::ApiBase)
                    .is_err()
            );
        }
    }

    #[test]
    fn endpoint_methods_resolve_only_fixed_paths() {
        let config = AuthConfig {
            authorization_endpoint: Url::parse(PRODUCTION_AUTHORIZE_URL).unwrap(),
            api_base: Url::parse("https://orcestr.com/api/v1/").unwrap(),
        };
        assert_eq!(
            config.native_login_endpoint().as_str(),
            "https://orcestr.com/api/v1/auth/token/login/"
        );
        assert_eq!(
            config.browser_login_endpoint().unwrap().as_str(),
            "https://orcestr.com/login"
        );
        assert_eq!(
            config.methods_endpoint().as_str(),
            "https://orcestr.com/api/v1/auth/methods/?origin=https%3A%2F%2Forcestr.com"
        );
        assert_eq!(
            config.oauth_token_endpoint().as_str(),
            "https://orcestr.com/api/v1/auth/oauth2/token/"
        );
        assert_eq!(
            config.legal_documents_endpoint("ru").as_str(),
            "https://orcestr.com/api/v1/legal/registration-documents/?language=ru"
        );
    }

    #[test]
    fn browser_login_endpoint_revalidates_authorization_origin_and_path() {
        let api_base = Url::parse("https://orcestr.com/api/v1/").unwrap();
        for value in [
            "https://attacker.test/oauth/authorize",
            "https://orcestr.com.attacker.test/oauth/authorize",
            "https://orcestr.com/oauth/authorize/",
            "https://orcestr.com/oauth/authorize?next=/admin",
            "https://user@orcestr.com/oauth/authorize",
        ] {
            let config = AuthConfig {
                authorization_endpoint: Url::parse(value).unwrap(),
                api_base: api_base.clone(),
            };
            assert!(config.browser_login_endpoint().is_err(), "{value}");
        }
    }

    #[test]
    fn explicit_debug_endpoints_share_browser_origin_for_provider_methods() {
        let config = AuthConfig {
            authorization_endpoint: Url::parse("http://localhost:8934/oauth/authorize").unwrap(),
            api_base: Url::parse("http://localhost:3934/api/v1/").unwrap(),
        };
        assert_eq!(
            config.browser_login_endpoint().unwrap().as_str(),
            "http://localhost:8934/login"
        );
        assert_eq!(
            config.methods_endpoint().as_str(),
            "http://localhost:3934/api/v1/auth/methods/?origin=http%3A%2F%2Flocalhost%3A8934"
        );
    }

    #[test]
    fn tauri_bundle_registers_the_exact_desktop_callback_scheme() {
        let tauri: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            tauri["identifier"].as_str(),
            Some("com.orcestr.realtranslate")
        );
        assert_eq!(
            tauri["plugins"]["deep-link"]["desktop"][0]["schemes"][0].as_str(),
            Some("com.orcestr.realtranslate")
        );
        assert!(REDIRECT_URI.starts_with("com.orcestr.realtranslate://"));
    }
}
