//! Telephony provider implementations.

pub mod mock;
pub mod plivo;
pub mod telnyx;
pub mod twilio;

pub use self::{
    mock::MockProvider, plivo::PlivoProvider, telnyx::TelnyxProvider, twilio::TwilioProvider,
};
