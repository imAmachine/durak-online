use super::{error, AppState};
use axum::{
    extract::{ConnectInfo, State},
    Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::json;
use ring::digest::{digest as ring_digest, SHA256};
use std::{collections::HashMap, net::{IpAddr, SocketAddr}, sync::Mutex, time::{Duration, Instant}};

const RATE_WINDOW: Duration = Duration::from_secs(300);
const RATE_LIMIT: usize = 10;
const MAX_RATE_IPS: usize = 4096;

#[derive(Default)]
struct RateLimiter {
    attempts: HashMap<IpAddr, Vec<Instant>>,
}

impl RateLimiter {
    fn allow(&mut self, ip: IpAddr, now: Instant) -> bool {
        let ip = match ip {
            IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(ip)),
            ip => ip,
        };
        self.attempts.retain(|_, attempts| {
            attempts.retain(|at| now.saturating_duration_since(*at) < RATE_WINDOW);
            !attempts.is_empty()
        });
        if !self.attempts.contains_key(&ip) && self.attempts.len() >= MAX_RATE_IPS {
            return false;
        }
        let attempts = self.attempts.entry(ip).or_default();
        if attempts.len() >= RATE_LIMIT {
            return false;
        }
        attempts.push(now);
        true
    }
}

pub struct Config {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    http: reqwest::Client,
    rate_limiter: Mutex<RateLimiter>,
}
pub struct Pending {
    verifier: String,
    secret_hash: String,
    expires: Instant,
}
impl Config {
    pub fn from_env() -> Result<Option<Self>, String> {
        let id = std::env::var("DISCORD_CLIENT_ID").unwrap_or_default();
        let secret = std::env::var("DISCORD_CLIENT_SECRET").unwrap_or_default();
        let redirect = std::env::var("DISCORD_REDIRECT_URI").unwrap_or_default();
        if id.is_empty() && secret.is_empty() && redirect.is_empty() {
            return Ok(None);
        }
        if id.is_empty() || secret.is_empty() || redirect.is_empty() {
            return Err(
                "Set DISCORD_CLIENT_ID, DISCORD_CLIENT_SECRET and DISCORD_REDIRECT_URI together"
                    .into(),
            );
        }
        let uri = url::Url::parse(&redirect).map_err(|_| "Invalid DISCORD_REDIRECT_URI")?;
        if uri.scheme() != "https"
            && !(uri.scheme() == "http"
                && matches!(uri.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
        {
            return Err("DISCORD_REDIRECT_URI must use HTTPS (HTTP is allowed on loopback)".into());
        }
        if uri.fragment().is_some() || !uri.username().is_empty() || uri.password().is_some() {
            return Err("Invalid DISCORD_REDIRECT_URI".into());
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "Cannot build Discord client")?;
        Ok(Some(Self {
            client_id: id,
            client_secret: secret,
            redirect_uri: redirect,
            http,
            rate_limiter: Mutex::new(RateLimiter::default()),
        }))
    }
}
fn random() -> Result<String, Response> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Источник случайных чисел недоступен",
        )
    })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
pub(crate) fn digest(s: &str) -> String {
    ring_digest(&SHA256, s.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_limit_is_per_ip_and_expires() {
        let mut limiter = RateLimiter::default();
        let now = Instant::now();
        let ip = "192.0.2.1".parse().unwrap();
        for _ in 0..RATE_LIMIT {
            assert!(limiter.allow(ip, now));
        }
        assert!(!limiter.allow(ip, now));
        assert!(!limiter.allow("::ffff:192.0.2.1".parse().unwrap(), now));
        assert!(limiter.allow("192.0.2.2".parse().unwrap(), now));
        assert!(limiter.allow(ip, now + RATE_WINDOW));
    }

    #[test]
    fn rate_limit_storage_is_bounded_and_recovers() {
        let mut limiter = RateLimiter::default();
        let now = Instant::now();
        for n in 0..MAX_RATE_IPS {
            assert!(limiter.allow(IpAddr::V6((n as u128).into()), now));
        }
        let extra = IpAddr::V6((MAX_RATE_IPS as u128).into());
        assert!(!limiter.allow(extra, now));
        assert!(limiter.allow(extra, now + RATE_WINDOW));
        assert_eq!(limiter.attempts.len(), 1);
    }

    #[test]
    fn generated_secrets_have_expected_encoding_and_are_distinct() {
        let a = random().unwrap();
        let b = random().unwrap();
        assert_eq!(a.len(), 43);
        assert_ne!(a, b);
        assert_eq!(digest(&a).len(), 64);
    }
}

pub async fn start(State(state): State<AppState>, peer: Option<Extension<ConnectInfo<SocketAddr>>>) -> Response {
    let (Some(config), Some(_)) = (&state.oauth, &state.repository) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Discord вход недоступен");
    };
    let Some(Extension(ConnectInfo(peer))) = peer else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Адрес подключения недоступен");
    };
    let allowed = config.rate_limiter.lock().map(|mut limiter| limiter.allow(peer.ip(), Instant::now())).unwrap_or(false);
    if !allowed {
        return error(StatusCode::TOO_MANY_REQUESTS, "Слишком много запросов");
    }
    let (Ok(key), Ok(secret), Ok(verifier)) = (random(), random(), random()) else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Источник случайных чисел недоступен",
        );
    };
    let challenge = URL_SAFE_NO_PAD.encode(ring_digest(&SHA256, verifier.as_bytes()).as_ref());
    let mut pending = state.oauth_pending.lock().await;
    pending.retain(|_, v| v.expires > Instant::now());
    if pending.len() >= 4096 {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Слишком много запросов");
    }
    pending.insert(
        key.clone(),
        Pending {
            verifier,
            secret_hash: digest(&secret),
            expires: Instant::now() + Duration::from_secs(300),
        },
    );
    let mut url = url::Url::parse("https://discord.com/oauth2/authorize").unwrap();
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &config.redirect_uri)
        .append_pair("scope", "identify")
        .append_pair("state", &key)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({"url":url.as_str(),"flowSecret":secret})),
    )
        .into_response()
}
#[derive(Deserialize)]
pub struct Completion {
    state: String,
    code: String,
    #[serde(rename = "flowSecret")]
    flow_secret: String,
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
}
#[derive(Deserialize)]
struct Identity {
    id: String,
}
pub async fn complete(State(state): State<AppState>, Json(body): Json<Completion>) -> Response {
    let (Some(config), Some(repository)) = (&state.oauth, &state.repository) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Discord вход недоступен");
    };
    if body.state.len() > 128 || body.code.len() > 2048 || body.flow_secret.len() > 128 {
        return error(StatusCode::BAD_REQUEST, "Неверный запрос");
    }
    let pending = {
        let mut flows = state.oauth_pending.lock().await;
        if !flows.get(&body.state).is_some_and(|p| {
            p.expires > Instant::now() && p.secret_hash == digest(&body.flow_secret)
        }) {
            return error(StatusCode::FORBIDDEN, "Недействительный вход");
        }
        flows.remove(&body.state).unwrap()
    };
    let response = config
        .http
        .post("https://discord.com/api/oauth2/token")
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", body.code.as_str()),
            ("redirect_uri", config.redirect_uri.as_str()),
            ("code_verifier", pending.verifier.as_str()),
        ])
        .send()
        .await;
    let token = match response {
        Ok(r) if r.status().is_success() => r.json::<Token>().await.ok(),
        _ => None,
    };
    let Some(token) = token else {
        return error(StatusCode::BAD_GATEWAY, "Ошибка авторизации Discord");
    };
    let response = config
        .http
        .get("https://discord.com/api/users/@me")
        .bearer_auth(token.access_token)
        .send()
        .await;
    let identity = match response {
        Ok(r) if r.status().is_success() => r.json::<Identity>().await.ok(),
        _ => None,
    };
    let Some(identity) = identity.filter(|i| {
        !i.id.is_empty() && i.id.len() <= 32 && i.id.bytes().all(|b| b.is_ascii_digit())
    }) else {
        return error(StatusCode::BAD_GATEWAY, "Ошибка получения профиля Discord");
    };
    let (Ok(client_id), Ok(token)) = (random(), random()) else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Источник случайных чисел недоступен",
        );
    };
    let client_id = format!("discord:{client_id}");
    let token = digest(&token);
    let mut sessions = state.sessions.write().await;
    let id = match repository
        .lock()
        .await
        .oauth_login(&identity.id, &client_id, &digest(&token))
    {
        Ok(id) => id,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно"),
    };
    sessions.insert(id.clone(), token.clone());
    if let Some(previous) = state.clients.write().await.remove(&id) {
        let _ = previous.sender.try_send(Ok(axum::response::sse::Event::default()
            .data(json!({"type":"revoked","reason":"session_rotated"}).to_string())));
    }
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({"clientId":id,"token":token})),
    )
        .into_response()
}
