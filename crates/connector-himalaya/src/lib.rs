//! Typed, read-only Himalaya v2 adapter for the connector snapshot store.

mod config;
mod connector;
mod error;
mod mime;
mod model;
mod runner;
mod tool;

pub use {
    config::{HimalayaAccountConfig, HimalayaBackend, HimalayaDatasetConfig},
    connector::HimalayaConnector,
    error::{HimalayaConnectorError, Result},
    mime::parse_message,
    model::{
        HimalayaAccount, HimalayaAddress, HimalayaAttachment, HimalayaEnvelope, HimalayaFlag,
        HimalayaIanaFlag, HimalayaMailbox, HimalayaMessageBody, HimalayaSnapshot, MimeStatus,
    },
    runner::{
        HimalayaInvocation, HimalayaOutput, HimalayaRunner, ProcessHimalayaRunner,
        validate_v2_version,
    },
    tool::HimalayaConnectorTool,
};
