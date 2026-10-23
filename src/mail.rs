use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

#[derive(Clone)]
pub struct Mailer {
    smtp: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    #[cfg(test)]
    pub outbox: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    #[cfg(test)]
    pub fail_delivery: bool,
}

impl Mailer {
    pub fn from_env() -> Self {
        let host = std::env::var("SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = std::env::var("SMTP_PORT")
            .unwrap_or_else(|_| "1025".into())
            .parse()
            .expect("Invalid SMTP_PORT");
        Self {
            // Local Mailpit only: no TLS, authentication, or external email service.
            smtp: AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
                .port(port)
                .build(),
            from: std::env::var("MAIL_FROM")
                .unwrap_or_else(|_| "Bound <hello@bound.test>".into())
                .parse()
                .expect("Invalid MAIL_FROM"),
            #[cfg(test)]
            outbox: None,
            #[cfg(test)]
            fail_delivery: false,
        }
    }

    pub async fn verify_email(
        &self,
        email: &str,
        school: &str,
        link: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let message = Message::builder()
            .from(self.from.clone()).to(email.parse()?)
            .subject("Verify your school email — Bound")
            .body(format!("Verify your {school} email address to use Bound:\n\n{link}\n\nThis link expires in 30 minutes. If you didn't request this email, ignore it."))?;
        #[cfg(test)]
        if self.fail_delivery {
            return Err(std::io::Error::other("Simulated SMTP failure").into());
        }
        #[cfg(test)]
        if let Some(outbox) = &self.outbox {
            outbox
                .lock()
                .unwrap()
                .push(String::from_utf8(message.formatted())?);
            return Ok(());
        }
        self.smtp.send(message).await?;
        Ok(())
    }
}
