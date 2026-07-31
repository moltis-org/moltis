//! Secure materialization of immutable Git revisions.

mod error;
mod materialize;
mod source;
mod transport;
mod tree;

pub use {
    error::Error,
    materialize::{MaterializationLimits, MaterializedRepository, Materializer},
    source::{
        Access, HttpsCredentials, HttpsSource, LocalSource, RepositorySource, RequestedRevision,
        SshCredentials, SshSource,
    },
    transport::{RepositoryBackend, SystemRepositoryBackend},
};
