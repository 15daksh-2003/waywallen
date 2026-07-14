//! Daemon-owned Steam sign-in via the `IAuthenticationService` QR flow: plain
//! HTTPS and protobuf, no Steam client and no interactive DepotDownloader login.
//! The refresh token is handed to DepotDownloader through its `account.config`;
//! the access token drives the workshop browse API.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use prost::Message;
use std::time::Duration;

mod pb {
    include!(concat!(env!("OUT_DIR"), "/waywallen.steam.rs"));
}

const BEGIN_URL: &str =
    "https://api.steampowered.com/IAuthenticationService/BeginAuthSessionViaQR/v1/";
const POLL_URL: &str =
    "https://api.steampowered.com/IAuthenticationService/PollAuthSessionStatus/v1/";
/// EAuthTokenPlatformType::SteamClient yields a refresh token DepotDownloader
/// can use for depot access.
const PLATFORM_STEAMCLIENT: i32 = 1;

/// A pending QR sign-in. Render `challenge_url` as a QR, then poll.
pub struct QrSession {
    pub client_id: u64,
    pub request_id: Vec<u8>,
    pub challenge_url: String,
    pub interval: Duration,
}

/// Tokens from a completed sign-in.
#[derive(Debug, Clone)]
pub struct Tokens {
    pub account_name: String,
    pub refresh_token: String,
    pub access_token: String,
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("build steam auth http client")
}

async fn service_post<Q: Message, R: Message + Default>(url: &str, req: &Q) -> Result<R> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(req.encode_to_vec());
    let resp = client()?
        .post(url)
        .form(&[("input_protobuf_encoded", encoded.as_str())])
        .send()
        .await
        .context("steam auth request failed")?;
    let eresult = resp
        .headers()
        .get("x-eresult")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let status = resp.status();
    let body = resp.bytes().await.context("read steam auth response")?;
    if !status.is_success() {
        bail!("steam auth HTTP {status} (eresult {eresult:?})");
    }
    R::decode(body).context("decode steam auth response")
}

pub async fn begin_qr() -> Result<QrSession> {
    let req = pb::BeginAuthSessionViaQrRequest {
        device_friendly_name: "waywallen".into(),
        platform_type: PLATFORM_STEAMCLIENT,
        website_id: "Client".into(),
        device_details: Some(pb::DeviceDetails {
            device_friendly_name: "waywallen".into(),
            platform_type: PLATFORM_STEAMCLIENT,
            os_type: 0,
        }),
    };
    let resp: pb::BeginAuthSessionViaQrResponse = service_post(BEGIN_URL, &req).await?;
    if resp.challenge_url.is_empty() {
        bail!("steam returned no QR challenge");
    }
    let interval = if resp.interval > 0.0 {
        Duration::from_secs_f32(resp.interval)
    } else {
        Duration::from_secs(5)
    };
    Ok(QrSession {
        client_id: resp.client_id,
        request_id: resp.request_id,
        challenge_url: resp.challenge_url,
        interval,
    })
}

/// Result of one poll of a pending QR session.
pub enum Poll {
    /// Still waiting for the user to scan and approve.
    Pending,
    /// Steam rotated the QR; the challenge URL was refreshed in `session` and
    /// must be re-rendered for the user to scan.
    Rotated,
    /// The user approved; sign-in is complete.
    Done(Tokens),
}

/// Poll once, advancing `session` if Steam rotates the QR (updates its client id
/// and challenge URL in place).
pub async fn poll_once(session: &mut QrSession) -> Result<Poll> {
    let req = pb::PollAuthSessionStatusRequest {
        client_id: session.client_id,
        request_id: session.request_id.clone(),
    };
    let resp: pb::PollAuthSessionStatusResponse = service_post(POLL_URL, &req).await?;
    if !resp.refresh_token.is_empty() {
        return Ok(Poll::Done(Tokens {
            account_name: resp.account_name,
            refresh_token: resp.refresh_token,
            access_token: resp.access_token,
        }));
    }
    if resp.new_client_id != 0 {
        session.client_id = resp.new_client_id;
    }
    if !resp.new_challenge_url.is_empty() && resp.new_challenge_url != session.challenge_url {
        session.challenge_url = resp.new_challenge_url;
        return Ok(Poll::Rotated);
    }
    Ok(Poll::Pending)
}

/// Render the challenge URL as a scannable QR, returned as an SVG `data:` URL
/// for a QML `Image`.
pub fn qr_data_url(challenge_url: &str) -> Result<String> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let code = QrCode::new(challenge_url.as_bytes()).context("build QR code")?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    ))
}

/// Serialize DepotDownloader's `AccountSettingsStore` (LoginTokens map) the way
/// protobuf-net expects, then raw-DEFLATE it (its `DeflateStream`).
pub fn encode_account_config(account_name: &str, refresh_token: &str) -> Result<Vec<u8>> {
    use std::io::Write as _;
    let store = pb::AccountSettingsStore {
        login_tokens: [(account_name.to_string(), refresh_token.to_string())]
            .into_iter()
            .collect(),
        guard_data: Default::default(),
    };
    let raw = store.encode_to_vec();
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&raw)
        .map_err(|e| anyhow!("deflate account.config: {e}"))?;
    enc.finish().map_err(|e| anyhow!("finish deflate: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_config_is_nonempty_deflate() {
        let bytes = encode_account_config("acct", "refresh").expect("encode");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn qr_data_url_is_svg() {
        let url = qr_data_url("https://s.team/q/1/123").expect("qr");
        assert!(url.starts_with("data:image/svg+xml;base64,"));
    }
}
