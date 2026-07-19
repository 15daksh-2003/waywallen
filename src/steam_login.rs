//! In-UI Steam sign-in via the daemon-owned QR flow (`steam_auth`): we ask Steam
//! to start a QR session, render the challenge URL ourselves, poll
//! IAuthenticationService until the user approves on their phone, then persist
//! the session and hand DepotDownloader a token via its `account.config`. No
//! DepotDownloader interactive login, no TTY, no kill race. One session at a time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::Notify;

use crate::fetcher::steam_workshop;
use crate::steam_auth::{self, Poll};
use crate::steam_session::{self, Session};
use crate::AppState;

const SCAN_DEADLINE: Duration = Duration::from_secs(180);

static LOGIN_ACTIVE: AtomicBool = AtomicBool::new(false);
static LOGIN_CANCEL: LazyLock<Notify> = LazyLock::new(Notify::new);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginUpdate {
    /// QR is ready; the string is an SVG `data:` URL for a QML `Image`.
    AwaitingScan(String),
    /// Signed in; the string is the account name.
    Success(String),
    Failed(String),
    Cancelled,
}

pub fn cancel() {
    LOGIN_CANCEL.notify_waiters();
}

pub fn is_signed_in() -> bool {
    steam_session::load().is_some()
}

pub fn account_name() -> Option<String> {
    steam_session::load().map(|s| s.account_name)
}

pub async fn sign_out() -> Result<()> {
    steam_session::clear().await?;
    steam_workshop::clear_token().await?;
    Ok(())
}

pub async fn start(state: &Arc<AppState>, mut emit: impl FnMut(LoginUpdate)) {
    if LOGIN_ACTIVE.swap(true, Ordering::SeqCst) {
        emit(LoginUpdate::Failed(
            "a Steam login is already in progress".into(),
        ));
        return;
    }
    let outcome = run(state, &LOGIN_CANCEL, &mut emit).await;
    if let Err(e) = outcome {
        emit(LoginUpdate::Failed(e.to_string()));
    }
    LOGIN_ACTIVE.store(false, Ordering::SeqCst);
}

async fn run(
    state: &Arc<AppState>,
    cancel: &Notify,
    emit: &mut impl FnMut(LoginUpdate),
) -> Result<()> {
    let mut session = steam_auth::begin_qr().await?;
    emit(LoginUpdate::AwaitingScan(steam_auth::qr_data_url(
        &session.challenge_url,
    )?));

    let deadline = tokio::time::sleep(SCAN_DEADLINE);
    tokio::pin!(deadline);
    let mut poll = tokio::time::interval(session.interval);
    // skip first tick
    poll.tick().await;

    let tokens = loop {
        tokio::select! {
            _ = cancel.notified() => {
                emit(LoginUpdate::Cancelled);
                return Ok(());
            }
            _ = &mut deadline => {
                emit(LoginUpdate::Failed("login timed out; try again".into()));
                return Ok(());
            }
            _ = poll.tick() => match steam_auth::poll_once(&mut session).await {
                Ok(Poll::Done(t)) => break t,
                Ok(Poll::Rotated) => {
                    emit(LoginUpdate::AwaitingScan(steam_auth::qr_data_url(
                        &session.challenge_url,
                    )?));
                }
                Ok(Poll::Pending) => {}
                Err(e) => {
                    emit(LoginUpdate::Failed(e.to_string()));
                    return Ok(());
                }
            },
        }
    };

    steam_session::save(&Session {
        account_name: tokens.account_name.clone(),
        refresh_token: tokens.refresh_token.clone(),
        access_token: tokens.access_token.clone(),
    })
    .await?;
    steam_workshop::write_token(state, &tokens.account_name, &tokens.refresh_token).await?;

    emit(LoginUpdate::Success(tokens.account_name));
    Ok(())
}
