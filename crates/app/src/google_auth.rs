//! Desktop-only Google authorization. Tokens remain in native memory or Keychain.

use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, RedirectUrl, RefreshToken, Scope, TokenResponse, TokenUrl,
    basic::BasicClient,
};
use reqwest::{Url, blocking::Client};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GMAIL_SCOPE: &str = "https://www.googleapis.com/auth/gmail.metadata";
const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.readonly";
const KEYCHAIN_SERVICE: &str = "Picket.Google";
const KEYCHAIN_USER: &str = "primary";

type GoogleClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid Google desktop OAuth client")]
    InvalidClient,
    #[error("Google authorization was cancelled, expired, or invalid")]
    Authorization,
    #[error("Google did not grant both required read-only scopes")]
    MissingScope,
    #[error("Google account identity changed")]
    ChangedAccount,
    #[error("Google token exchange failed")]
    Token,
    #[error("macOS Keychain is unavailable")]
    Keychain,
    #[error("Google profile could not be verified")]
    Profile,
    #[error("network client could not start")]
    Network,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub account_id: String,
    pub account_email: String,
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

pub struct Session {
    pub credentials: Credentials,
    pub access_token: String,
    pub expires_at: Instant,
}

pub fn http_client() -> Result<Client, AuthError> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| AuthError::Network)
}

pub fn connect(client_id: &str, client_secret: &str) -> Result<Session, AuthError> {
    if client_id.is_empty()
        || client_id.len() > 512
        || client_id.chars().any(char::is_control)
        || client_secret.len() > 512
        || client_secret.chars().any(char::is_control)
    {
        return Err(AuthError::InvalidClient);
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| AuthError::Network)?;
    let port = listener
        .local_addr()
        .map_err(|_| AuthError::Network)?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/oauth/callback");
    let oauth = oauth_client(client_id, client_secret, &redirect)?;
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (authorization_url, state) = oauth
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new(GMAIL_SCOPE.into()))
        .add_scope(Scope::new(CALENDAR_SCOPE.into()))
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .set_pkce_challenge(challenge)
        .url();
    let launched = Command::new("/usr/bin/open")
        .arg(authorization_url.as_str())
        .status()
        .map_err(|_| AuthError::Authorization)?;
    if !launched.success() {
        return Err(AuthError::Authorization);
    }
    let code = receive_code(listener, port, state.secret())?;
    let token = oauth
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(verifier)
        .request(&http_client()?)
        .map_err(|_| AuthError::Token)?;
    check_scopes(token.scopes())?;
    let refresh_token = token
        .refresh_token()
        .ok_or(AuthError::Token)?
        .secret()
        .clone();
    let access_token = token.access_token().secret().clone();
    let expires_at = expiry(token.expires_in());
    let account_email = verified_profile(&http_client()?, &access_token)?;
    let account_id =
        existing_account_id(&account_email)?.unwrap_or_else(|| Uuid::new_v4().to_string());
    let credentials = Credentials {
        account_id,
        account_email,
        client_id: client_id.into(),
        client_secret: client_secret.into(),
        refresh_token,
    };
    save(&credentials)?;
    Ok(Session {
        credentials,
        access_token,
        expires_at,
    })
}

fn existing_account_id(email: &str) -> Result<Option<String>, AuthError> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).map_err(|_| AuthError::Keychain)?;
    match entry.get_password() {
        Ok(value) => {
            let prior: Credentials =
                serde_json::from_str(&value).map_err(|_| AuthError::Keychain)?;
            Ok((prior.account_email == email).then_some(prior.account_id))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(AuthError::Keychain),
    }
}

pub fn restore() -> Result<Option<Session>, AuthError> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).map_err(|_| AuthError::Keychain)?;
    let stored = match entry.get_password() {
        Ok(value) => value,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(_) => return Err(AuthError::Keychain),
    };
    let mut credentials: Credentials =
        serde_json::from_str(&stored).map_err(|_| AuthError::Keychain)?;
    let oauth = oauth_client(
        &credentials.client_id,
        &credentials.client_secret,
        "http://127.0.0.1/",
    )?;
    let token = oauth
        .exchange_refresh_token(&RefreshToken::new(credentials.refresh_token.clone()))
        .request(&http_client()?)
        .map_err(refresh_error)?;
    if token.scopes().is_some() {
        check_scopes(token.scopes())?;
    }
    let access_token = token.access_token().secret().clone();
    let expires_at = expiry(token.expires_in());
    if let Some(rotated) = token.refresh_token() {
        credentials.refresh_token = rotated.secret().clone();
        save(&credentials)?;
    }
    let account_email = verified_profile(&http_client()?, &access_token)?;
    if account_email != credentials.account_email {
        return Err(AuthError::ChangedAccount);
    }
    Ok(Some(Session {
        credentials,
        access_token,
        expires_at,
    }))
}

pub fn refresh(session: &mut Session) -> Result<(), AuthError> {
    if Instant::now() < session.expires_at {
        return Ok(());
    }
    let oauth = oauth_client(
        &session.credentials.client_id,
        &session.credentials.client_secret,
        "http://127.0.0.1/",
    )?;
    let token = oauth
        .exchange_refresh_token(&RefreshToken::new(
            session.credentials.refresh_token.clone(),
        ))
        .request(&http_client()?)
        .map_err(refresh_error)?;
    if token.scopes().is_some() {
        check_scopes(token.scopes())?;
    }
    if let Some(rotated) = token.refresh_token() {
        session.credentials.refresh_token = rotated.secret().clone();
        save(&session.credentials)?;
    }
    session.access_token = token.access_token().secret().clone();
    session.expires_at = expiry(token.expires_in());
    Ok(())
}

/// A rejected grant needs the user to reconnect; anything else is a transient failure.
fn refresh_error<E: std::error::Error + 'static, T: oauth2::ErrorResponse>(
    error: oauth2::RequestTokenError<E, T>,
) -> AuthError {
    match error {
        oauth2::RequestTokenError::ServerResponse(_) => AuthError::Authorization,
        _ => AuthError::Network,
    }
}

fn expiry(duration: Option<Duration>) -> Instant {
    Instant::now()
        + duration
            .unwrap_or(Duration::from_secs(1800))
            .saturating_sub(Duration::from_secs(60))
}

fn oauth_client(
    client_id: &str,
    client_secret: &str,
    redirect: &str,
) -> Result<GoogleClient, AuthError> {
    let auth = AuthUrl::new(AUTH_URL.into()).map_err(|_| AuthError::InvalidClient)?;
    let token = TokenUrl::new(TOKEN_URL.into()).map_err(|_| AuthError::InvalidClient)?;
    let redirect = RedirectUrl::new(redirect.into()).map_err(|_| AuthError::InvalidClient)?;
    let client = BasicClient::new(ClientId::new(client_id.into()))
        .set_auth_uri(auth)
        .set_token_uri(token)
        .set_redirect_uri(redirect);
    Ok(if client_secret.is_empty() {
        client
    } else {
        client.set_client_secret(ClientSecret::new(client_secret.into()))
    })
}

fn receive_code(
    listener: TcpListener,
    port: u16,
    expected_state: &str,
) -> Result<String, AuthError> {
    listener
        .set_nonblocking(true)
        .map_err(|_| AuthError::Network)?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut socket, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .map_err(|_| AuthError::Network)?;
                let mut bytes = [0_u8; 4096];
                let mut count = 0;
                while count < bytes.len()
                    && !bytes[..count].windows(4).any(|part| part == b"\r\n\r\n")
                {
                    match socket.read(&mut bytes[count..]) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => count += read,
                    }
                }
                let Ok(request) = std::str::from_utf8(&bytes[..count]) else {
                    continue;
                };
                let Ok(code) = parse_callback(request, port, expected_state) else {
                    continue;
                };
                let body = "Google connected. Return to the Picket app.";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                socket
                    .write_all(response.as_bytes())
                    .map_err(|_| AuthError::Authorization)?;
                return Ok(code);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return Err(AuthError::Network),
        }
    }
    Err(AuthError::Authorization)
}

fn parse_callback(request: &str, port: u16, expected_state: &str) -> Result<String, AuthError> {
    let line = request.lines().next().ok_or(AuthError::Authorization)?;
    let path = line
        .strip_prefix("GET ")
        .and_then(|line| line.strip_suffix(" HTTP/1.1"))
        .ok_or(AuthError::Authorization)?;
    if !path.starts_with("/oauth/callback?") || path.contains('#') {
        return Err(AuthError::Authorization);
    }
    let host = request
        .lines()
        .find_map(|line| line.strip_prefix("Host: ").map(str::trim_end));
    if host != Some(format!("127.0.0.1:{port}").as_str()) {
        return Err(AuthError::Authorization);
    }
    let url = Url::parse(&format!("http://127.0.0.1:{port}{path}"))
        .map_err(|_| AuthError::Authorization)?;
    if url.path() != "/oauth/callback" {
        return Err(AuthError::Authorization);
    }
    let mut code = None;
    let mut state = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" if code.is_none() => code = Some(value.into_owned()),
            "state" if state.is_none() => state = Some(value.into_owned()),
            "code" | "state" => return Err(AuthError::Authorization),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err(AuthError::Authorization);
    }
    code.filter(|code: &String| !code.is_empty())
        .ok_or(AuthError::Authorization)
}

fn check_scopes(scopes: Option<&Vec<Scope>>) -> Result<(), AuthError> {
    let scopes = scopes.ok_or(AuthError::MissingScope)?;
    if scopes.len() != 2
        || !scopes.iter().any(|scope| scope.as_ref() == GMAIL_SCOPE)
        || !scopes.iter().any(|scope| scope.as_ref() == CALENDAR_SCOPE)
    {
        return Err(AuthError::MissingScope);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    email_address: String,
    history_id: String,
}

fn verified_profile(client: &Client, access_token: &str) -> Result<String, AuthError> {
    let response = client
        .get("https://gmail.googleapis.com/gmail/v1/users/me/profile")
        .bearer_auth(access_token)
        .send()
        .map_err(|_| AuthError::Profile)?;
    if !response.status().is_success() || response.content_length().is_some_and(|size| size > 8192)
    {
        return Err(AuthError::Profile);
    }
    let mut bytes = Vec::new();
    response
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| AuthError::Profile)?;
    if bytes.len() > 8192 {
        return Err(AuthError::Profile);
    }
    let profile: Profile = serde_json::from_slice(&bytes).map_err(|_| AuthError::Profile)?;
    if profile.email_address.len() > 320
        || !profile.email_address.contains('@')
        || profile.history_id.is_empty()
    {
        return Err(AuthError::Profile);
    }
    Ok(profile.email_address.to_lowercase())
}

fn save(credentials: &Credentials) -> Result<(), AuthError> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).map_err(|_| AuthError::Keychain)?;
    let value = serde_json::to_string(credentials).map_err(|_| AuthError::Keychain)?;
    entry.set_password(&value).map_err(|_| AuthError::Keychain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_requires_exact_state_and_one_code() {
        let valid = "GET /oauth/callback?state=expected&code=synthetic HTTP/1.1\r\nHost: 127.0.0.1:4567\r\n\r\n";
        assert_eq!(
            parse_callback(valid, 4567, "expected").ok().as_deref(),
            Some("synthetic")
        );
        assert!(parse_callback(valid, 4567, "wrong").is_err());
        assert!(
            parse_callback(
                "GET /oauth/callback?state=expected&code=a&code=b HTTP/1.1",
                4567,
                "expected"
            )
            .is_err()
        );
    }

    #[test]
    fn granted_scopes_must_be_exact() {
        let allowed = vec![
            Scope::new(GMAIL_SCOPE.into()),
            Scope::new(CALENDAR_SCOPE.into()),
        ];
        assert!(check_scopes(Some(&allowed)).is_ok());
        assert!(check_scopes(None).is_err());
        assert!(check_scopes(Some(&allowed[..1].to_vec())).is_err());
    }
}
