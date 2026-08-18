//! Typed, read-only Gmail adapter for the connector snapshot store.

mod client;
mod config;
mod connector;
mod error;
mod model;
mod tool;

pub use {
    client::{GmailClient, NativeGmailClient},
    config::{AuthorizedUserToken, GmailAccountConfig, GmailDatasetConfig},
    connector::GmailConnector,
    error::{GmailConnectorError, Result},
    model::{
        GmailApiMessage, GmailAttachment, GmailBodyFormat, GmailHeader, GmailMessageBody,
        GmailMessagePage, GmailMessagePart, GmailMessagePartBody, GmailMessageRef, GmailProfile,
        GmailSnapshot,
    },
    tool::{GmailConnectorTool, register_gmail_connector_tool},
};
