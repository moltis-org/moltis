//! Provider registry: model registration, lookup, discovery, and lifecycle.

mod core;
pub mod registration;
#[cfg(test)]
mod tests;

pub use self::core::*;
