//! NetBird private mesh integration.

pub mod error;
mod manager;

pub use {
    error::{Error, Result},
    manager::*,
};
