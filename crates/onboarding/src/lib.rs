//! Interactive onboarding wizard.
//!
//! Flow: welcome → user name → agent name → agent theme → confirm → done.

pub mod error;
pub mod service;
pub mod state;
pub mod wizard;

pub use error::{Context, Error, Result};
