//! Shared Signal account state.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};

use tokio_util::sync::CancellationToken;

use crate::{client::SignalClient, config::SignalAccountConfig};

pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

pub struct AccountState {
    pub client: SignalClient,
    pub config: Arc<RwLock<SignalAccountConfig>>,
    pub cancel: CancellationToken,
    pub otp: Arc<Mutex<moltis_channels::otp::OtpState>>,
}
