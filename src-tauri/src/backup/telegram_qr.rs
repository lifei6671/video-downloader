//! Telegram's export/update/import QR authorization flow, independent of UI and session storage.
use super::telegram_session::{bounded, Account};
use anyhow::{bail, Result};
use base64::Engine;
use grammers_client::{session::updates::UpdatesLike, tl, InvocationError};
use qrcode::{Color, QrCode};
use std::{
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;
use zeroize::Zeroizing;

pub(super) const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) enum Response {
    Token {
        token: Zeroizing<Vec<u8>>,
        expires: i64,
    },
    Migrate {
        dc: i32,
        token: Zeroizing<Vec<u8>>,
    },
    Success,
    Password,
}
pub(super) trait Transport {
    fn export(&self) -> impl Future<Output = Result<Response>> + Send;
    fn import(&self, dc: i32, token: &[u8]) -> impl Future<Output = Result<Response>> + Send;
    fn account(&self) -> impl Future<Output = Result<Account>> + Send;
    fn scan_update(&self) -> impl Future<Output = Result<()>> + Send;
}
pub(super) struct DisplayQr {
    pub matrix: Vec<Vec<bool>>,
    pub expires: i64,
}
pub(super) enum Outcome {
    Ready(Account),
    Password,
}

pub(super) fn response(
    result: std::result::Result<tl::enums::auth::LoginToken, InvocationError>,
) -> Result<Response> {
    match result {
        Ok(tl::enums::auth::LoginToken::Token(token)) => Ok(Response::Token {
            token: Zeroizing::new(token.token),
            expires: i64::from(token.expires),
        }),
        Ok(tl::enums::auth::LoginToken::MigrateTo(token)) => Ok(Response::Migrate {
            dc: token.dc_id,
            token: Zeroizing::new(token.token),
        }),
        Ok(tl::enums::auth::LoginToken::Success(token)) => {
            if !matches!(
                token.authorization,
                tl::enums::auth::Authorization::Authorization(_)
            ) {
                bail!("TELEGRAM_SIGN_UP_REQUIRED");
            }
            Ok(Response::Success)
        }
        Err(InvocationError::Rpc(error)) if error.name == "SESSION_PASSWORD_NEEDED" => {
            Ok(Response::Password)
        }
        Err(error) => Err(super::telegram_session::rpc_error(error)),
    }
}

pub(super) fn is_login_update(update: UpdatesLike) -> bool {
    match update {
        UpdatesLike::Updates(tl::enums::Updates::UpdateShort(update)) => {
            matches!(update.update, tl::enums::Update::LoginToken)
        }
        UpdatesLike::Updates(tl::enums::Updates::Updates(updates)) => updates
            .updates
            .iter()
            .any(|u| matches!(u, tl::enums::Update::LoginToken)),
        UpdatesLike::Updates(tl::enums::Updates::Combined(updates)) => updates
            .updates
            .iter()
            .any(|u| matches!(u, tl::enums::Update::LoginToken)),
        _ => false,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn matrix(token: &[u8]) -> Result<Vec<Vec<bool>>> {
    let mut uri = Zeroizing::new(String::from("tg://login?token="));
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode_string(token, &mut uri);
    let code = QrCode::new(uri.as_bytes()).map_err(|_| anyhow::anyhow!("TELEGRAM_QR_INVALID"))?;
    // Four modules of quiet zone are part of the matrix, so every renderer keeps them intact.
    let side = code.width();
    let mut matrix = vec![vec![false; side + 8]; side + 8];
    for y in 0..side {
        for x in 0..side {
            matrix[y + 4][x + 4] = code[(x, y)] == Color::Dark;
        }
    }
    Ok(matrix)
}

pub(super) async fn run(
    transport: &impl Transport,
    cancel: &mut watch::Receiver<bool>,
    mut show: impl FnMut(Option<DisplayQr>),
) -> Result<Outcome> {
    let mut response = bounded(cancel, transport.export()).await?;
    loop {
        match response {
            Response::Token { token, expires } => {
                if expires > now() {
                    show(Some(DisplayQr {
                        matrix: matrix(&token)?,
                        expires,
                    }));
                    tokio::select! {
                        biased;
                        _ = cancel.wait_for(|v| *v) => bail!("TELEGRAM_CANCELED"),
                        result = transport.scan_update() => result?,
                        _ = tokio::time::sleep(Duration::from_secs(expires.saturating_sub(now()).max(0) as u64)) => {},
                    }
                }
                show(None);
                response = bounded(cancel, transport.export()).await?;
            }
            Response::Migrate { dc, token } => {
                show(None);
                response = bounded(cancel, transport.import(dc, &token)).await?;
            }
            Response::Success => {
                return Ok(Outcome::Ready(bounded(cancel, transport.account()).await?))
            }
            Response::Password => return Ok(Outcome::Password),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
    };
    struct Mock {
        replies: Mutex<VecDeque<Response>>,
        calls: Mutex<Vec<String>>,
        updates: AtomicUsize,
        scan_allowed: bool,
    }
    impl Transport for Mock {
        async fn export(&self) -> Result<Response> {
            self.calls.lock().unwrap().push("export".into());
            Ok(self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("fixture export response"))
        }
        async fn import(&self, dc: i32, token: &[u8]) -> Result<Response> {
            assert_eq!(dc, 4);
            assert_eq!(token, &[4, 2]);
            self.calls.lock().unwrap().push("import:4".into());
            Ok(self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("fixture import response"))
        }
        async fn account(&self) -> Result<Account> {
            self.calls.lock().unwrap().push("account".into());
            Ok(Account {
                id: "123".into(),
                name: "fixture".into(),
            })
        }
        async fn scan_update(&self) -> Result<()> {
            self.updates.fetch_add(1, Ordering::SeqCst);
            if !self.scan_allowed || self.replies.lock().unwrap().is_empty() {
                std::future::pending().await
            } else {
                Ok(())
            }
        }
    }
    #[tokio::test]
    async fn telegram_qr_scan_refreshes_then_migrates_before_confirming_account() {
        let mock = Mock {
            replies: Mutex::new(VecDeque::from([
                Response::Token {
                    token: Zeroizing::new(vec![1, 2, 3]),
                    expires: now() + 30,
                },
                Response::Migrate {
                    dc: 4,
                    token: Zeroizing::new(vec![4, 2]),
                },
                Response::Success,
            ])),
            calls: Mutex::new(vec![]),
            updates: AtomicUsize::new(0),
            scan_allowed: true,
        };
        let (_tx, mut cancel) = watch::channel(false);
        let mut shown = vec![];
        let result = run(&mock, &mut cancel, |qr| shown.push(qr)).await.unwrap();
        assert!(matches!(result,Outcome::Ready(Account{id,..}) if id=="123"));
        assert_eq!(
            *mock.calls.lock().unwrap(),
            ["export", "export", "import:4", "account"]
        );
        let qr = shown[0].as_ref().unwrap();
        assert!(qr.matrix.len() > 8 && qr.matrix.iter().all(|row| row.len() == qr.matrix.len()));
        assert!(qr.matrix[..4].iter().flatten().all(|value| !*value));
        assert!(
            shown.last().unwrap().is_none(),
            "consumed token is removed from display"
        );
    }
    #[tokio::test]
    async fn telegram_qr_expired_token_is_never_displayed_and_2fa_is_preserved() {
        let mock = Mock {
            replies: Mutex::new(VecDeque::from([
                Response::Token {
                    token: Zeroizing::new(vec![1]),
                    expires: now() - 1,
                },
                Response::Password,
            ])),
            calls: Mutex::new(vec![]),
            updates: AtomicUsize::new(0),
            scan_allowed: true,
        };
        let (_tx, mut cancel) = watch::channel(false);
        let mut displayed = false;
        assert!(matches!(
            run(&mock, &mut cancel, |qr| displayed |= qr.is_some())
                .await
                .unwrap(),
            Outcome::Password
        ));
        assert!(!displayed);
        assert_eq!(mock.updates.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn telegram_qr_cancel_stops_waiting_and_never_confirms_an_account() {
        let mock = Mock {
            replies: Mutex::new(VecDeque::from([Response::Token {
                token: Zeroizing::new(vec![1]),
                expires: now() + 30,
            }])),
            calls: Mutex::new(vec![]),
            updates: AtomicUsize::new(0),
            scan_allowed: true,
        };
        let (tx, mut cancel) = watch::channel(false);
        let result = run(&mock, &mut cancel, |qr| {
            if qr.is_some() {
                tx.send_replace(true);
            }
        })
        .await;
        assert_eq!(result.err().unwrap().to_string(), "TELEGRAM_CANCELED");
        assert_eq!(*mock.calls.lock().unwrap(), ["export"]);
    }
    #[tokio::test]
    async fn telegram_qr_expiry_without_update_refreshes_and_discards_the_old_code() {
        let mock = Mock {
            replies: Mutex::new(VecDeque::from([
                Response::Token {
                    token: Zeroizing::new(vec![1]),
                    expires: now() + 2,
                },
                Response::Success,
            ])),
            calls: Mutex::new(vec![]),
            updates: AtomicUsize::new(0),
            scan_allowed: false,
        };
        let (_tx, mut cancel) = watch::channel(false);
        let mut shown = vec![];
        let outcome = tokio::time::timeout(
            Duration::from_secs(4),
            run(&mock, &mut cancel, |qr| shown.push(qr.is_some())),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(outcome, Outcome::Ready(_)));
        assert_eq!(shown, [true, false]);
        assert_eq!(*mock.calls.lock().unwrap(), ["export", "export", "account"]);
    }
    #[test]
    fn telegram_qr_accepts_raw_login_updates_and_ignores_unrelated_containers() {
        assert!(is_login_update(UpdatesLike::Updates(
            tl::enums::Updates::UpdateShort(tl::types::UpdateShort {
                update: tl::enums::Update::LoginToken,
                date: 0
            })
        )));
        assert!(!is_login_update(UpdatesLike::Updates(
            tl::enums::Updates::TooLong
        )));
    }
}
