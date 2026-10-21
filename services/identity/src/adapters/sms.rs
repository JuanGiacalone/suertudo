//! The SMS channel.
//!
//! The provider is not decided yet: the flow of the takeover of a device is
//! built against the port of `crate::ports`, and the only implementation
//! that exists writes the message to the log. It is a development sender,
//! and the configuration refuses to start the worker with it outside
//! development: nobody deploys a service that writes the codes of its users
//! in the log.

use crate::ports::{BoxFuture, SmsError, SmsMessage, SmsSender};

/// The sender of development: it writes the message to the log.
///
/// The code of a verification has to reach the participant, and in
/// development there is no provider: the log is the channel.
#[derive(Debug, Default)]
pub struct LoggingSmsSender;

impl SmsSender for LoggingSmsSender {
    fn send(&self, to: &str, message: &SmsMessage) -> BoxFuture<'_, Result<(), SmsError>> {
        // The future owns what it logs: the lifetime of the future is the
        // one of the sender, not the one of the caller.
        let recipient = to.to_string();
        let body = message.body.clone();
        Box::pin(async move {
            tracing::warn!(
                recipient = %recipient,
                body = %body,
                "the SMS was not sent: this is the sender of development"
            );
            Ok(())
        })
    }
}
