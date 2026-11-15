//! Outgoing email. Only a logging mailer exists so far; a real transport (SMTP or an
//! HTTP mail API) plugs in behind the same trait before launch.

use std::sync::Mutex;

pub trait Mailer: Send + Sync + 'static {
    /// Must not block: a real implementation hands the message to a background task.
    fn send_verification(&self, to: &str, name: &str, link: &str);
}

/// Development mailer: writes the verification link to the log.
pub struct LogMailer;

impl Mailer for LogMailer {
    fn send_verification(&self, to: &str, name: &str, link: &str) {
        tracing::info!(to, name, link, "verification email (not sent: log mailer)");
    }
}

/// Test mailer: keeps every link.
#[derive(Default)]
pub struct CaptureMailer {
    pub sent: Mutex<Vec<(String, String)>>,
}

impl CaptureMailer {
    /// The last link sent to `to`.
    pub fn link_for(&self, to: &str) -> Option<String> {
        let sent = self.sent.lock().unwrap();
        sent.iter()
            .rev()
            .find(|(t, _)| t == to)
            .map(|(_, l)| l.clone())
    }
}

impl Mailer for CaptureMailer {
    fn send_verification(&self, to: &str, _name: &str, link: &str) {
        self.sent
            .lock()
            .unwrap()
            .push((to.to_owned(), link.to_owned()));
    }
}
