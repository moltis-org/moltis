use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};

use {moltis_channels::message_log::MessageLog, tokio_util::sync::CancellationToken};

use crate::{config::MatrixAccountConfig, otp::OtpState, outbound::MatrixOutbound};

pub struct AccountState {
    pub client: matrix_sdk::Client,
    pub account_id: String,
    pub config: MatrixAccountConfig,
    pub outbound: Arc<MatrixOutbound>,
    pub cancel: CancellationToken,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn moltis_channels::ChannelEventSink>>,
    pub otp: Mutex<OtpState>,
}

pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;
