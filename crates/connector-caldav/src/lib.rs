//! Typed CalDAV adapter for the connector snapshot store.

mod config;
mod connector;
mod error;
mod model;

pub use {
    config::{
        CalDavAccountConfig, CalDavAccountConfigView, CalDavDatasetConfig, CalDavFilters,
        CalendarSelection,
    },
    connector::CalDavConnector,
    error::{CalDavConnectorError, Result},
    model::{CalDavSnapshot, CalendarBody, EventBody, ParseStatus},
};
