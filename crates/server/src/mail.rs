//! Outgoing email: SMTP in production (`SMTP_URL`), the log in development.

use std::sync::{Arc, Mutex};

use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub body: String,
}

pub trait Mailer: Send + Sync + 'static {
    /// Must not block or fail the request: implementations send in the background.
    fn send(&self, email: Email);
}

pub fn verification(to: &str, name: &str, link: &str) -> Email {
    Email {
        to: to.to_owned(),
        subject: "Confirm your Pokerladder email".into(),
        body: format!(
            "Hi {name},\n\n\
             Confirm your email address to start playing on the ladder:\n\n\
             {link}\n\n\
             The link works once and expires in 48 hours. If you didn't sign up, ignore \
             this email.\n"
        ),
    }
}

/// Sent instead of an error when someone registers with an email that already has an
/// account, so the sign-up form doesn't reveal which emails are registered.
pub fn already_registered(to: &str, login_url: &str) -> Email {
    Email {
        to: to.to_owned(),
        subject: "Pokerladder sign-up attempt".into(),
        body: format!(
            "Someone (hopefully you) tried to create a Pokerladder account with this \
             email address, but it already has one.\n\n\
             Log in here with your name or email: {login_url}\n\n\
             If this wasn't you, you can ignore this email; nothing has changed.\n"
        ),
    }
}

pub fn password_reset(to: &str, name: &str, link: &str) -> Email {
    Email {
        to: to.to_owned(),
        subject: "Reset your Pokerladder password".into(),
        body: format!(
            "Hi {name},\n\n\
             Someone (hopefully you) asked to reset the password for your Pokerladder \
             account. Choose a new one here:\n\n\
             {link}\n\n\
             The link works once and expires in one hour. If you didn't ask for this, \
             ignore this email; your password hasn't changed.\n"
        ),
    }
}

/// Development mailer: writes the email to the log.
pub struct LogMailer;

impl Mailer for LogMailer {
    fn send(&self, email: Email) {
        tracing::info!(
            to = email.to,
            subject = email.subject,
            body = email.body,
            "email (log mailer; not sent)"
        );
    }
}

/// Sends through an SMTP relay, e.g. `smtps://user:pass@smtp.example.com:465`.
/// SMTP connections open at once; further emails wait their turn.
const MAX_CONCURRENT_SENDS: usize = 8;

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    sending: Arc<tokio::sync::Semaphore>,
}

impl SmtpMailer {
    pub fn new(smtp_url: &str, from: &str) -> anyhow::Result<SmtpMailer> {
        Ok(SmtpMailer {
            transport: AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp_url)?.build(),
            from: from.parse()?,
            sending: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SENDS)),
        })
    }

    /// Checks the server accepts our connection and credentials.
    pub async fn test_connection(&self) -> anyhow::Result<bool> {
        Ok(self.transport.test_connection().await?)
    }
}

impl Mailer for SmtpMailer {
    fn send(&self, email: Email) {
        let message = email
            .to
            .parse::<Mailbox>()
            .map_err(anyhow::Error::from)
            .and_then(|to| {
                Ok(Message::builder()
                    .from(self.from.clone())
                    .to(to)
                    .subject(email.subject)
                    .body(email.body)?)
            });
        let message = match message {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("could not build email: {e:#}");
                return;
            }
        };
        let transport = self.transport.clone();
        let sending = self.sending.clone();
        tokio::spawn(async move {
            let Ok(_permit) = sending.acquire_owned().await else {
                return;
            };
            if let Err(e) = transport.send(message).await {
                tracing::error!("sending email failed: {e}");
            }
        });
    }
}

/// Test mailer: keeps every email.
#[derive(Default)]
pub struct CaptureMailer {
    pub sent: Mutex<Vec<Email>>,
}

impl CaptureMailer {
    /// The last email sent to `to`.
    pub fn last_to(&self, to: &str) -> Option<Email> {
        let sent = self.sent.lock().unwrap();
        sent.iter().rev().find(|e| e.to == to).cloned()
    }

    /// The token from the last verification link sent to `to`.
    pub fn token_for(&self, to: &str) -> Option<String> {
        let body = self.last_to(to)?.body;
        let start = body.find("token=")? + "token=".len();
        let token: String = body[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        Some(token)
    }
}

impl Mailer for CaptureMailer {
    fn send(&self, email: Email) {
        self.sent.lock().unwrap().push(email);
    }
}
