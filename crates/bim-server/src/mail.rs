//! Outgoing email (password-reset links). SMTP via lettre, configured by `SMTP_URL` + `MAIL_FROM`;
//! any provider with an SMTP endpoint works (Resend, Postmark, SendGrid, SES, Gmail, ...).

use lettre::message::header::ContentType;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::fmt;
use std::future::Future;
use std::pin::Pin;

pub type SendFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Sends one plain-text message. Implemented by `SmtpMailer`; tests substitute a recorder.
pub trait Mailer: Send + Sync + fmt::Debug {
    fn send<'a>(&'a self, to: &'a str, subject: &'a str, body: &'a str) -> SendFuture<'a>;
}

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: lettre::message::Mailbox,
}

impl fmt::Debug for SmtpMailer {
    // the transport holds credentials; never print it
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SmtpMailer(from {})", self.from)
    }
}

impl SmtpMailer {
    /// `url` like `smtps://user:pass@smtp.example.com:465` or `smtp://...:587?tls=required`;
    /// `from` like `buildsim <no-reply@example.com>`.
    pub fn new(url: &str, from: &str) -> Result<Self, String> {
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .map_err(|e| format!("SMTP_URL: {e}"))?
            .build();
        let from = from.parse().map_err(|e| format!("MAIL_FROM: {e}"))?;
        Ok(SmtpMailer { transport, from })
    }

    /// From `SMTP_URL` and `MAIL_FROM`; `None` (reset disabled) unless both are set.
    pub fn from_env() -> Option<Self> {
        let var = |k| {
            std::env::var(k)
                .ok()
                .filter(|v: &String| !v.trim().is_empty())
        };
        match (var("SMTP_URL"), var("MAIL_FROM")) {
            (Some(url), Some(from)) => match SmtpMailer::new(&url, &from) {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::error!("email disabled: {e}");
                    None
                }
            },
            (None, None) => None,
            _ => {
                tracing::warn!("email disabled: set both SMTP_URL and MAIL_FROM");
                None
            }
        }
    }
}

impl Mailer for SmtpMailer {
    fn send<'a>(&'a self, to: &'a str, subject: &'a str, body: &'a str) -> SendFuture<'a> {
        Box::pin(async move {
            let msg = Message::builder()
                .from(self.from.clone())
                .to(to.parse().map_err(|e| format!("recipient: {e}"))?)
                .subject(subject)
                .header(ContentType::TEXT_PLAIN)
                .body(body.to_owned())
                .map_err(|e| e.to_string())?;
            self.transport
                .send(msg)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    }
}
