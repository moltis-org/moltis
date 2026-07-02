//! Persistent storage backend using sled (embedded key-value database).
//!
//! Replaces `MemoryStore` so that Signal Protocol session state survives
//! restarts — users don't need to re-scan the QR code every time.
//!
//! Each account gets its own sled database at `<data_dir>/whatsapp/<account_id>/`.

use std::{fmt::Write, path::Path, sync::atomic::AtomicI32};

use {
    async_trait::async_trait,
    bytes::Bytes,
    serde::{Serialize, de::DeserializeOwned},
    tracing::{debug, info, warn},
    wacore::{
        appstate::{hash::HashState, processor::AppStateMutationMAC},
        store::{
            error::{Result, StoreError},
            traits::*,
        },
    },
};

/// Wrap a sled error as a `StoreError::Database`.
fn db_err(e: sled::Error) -> StoreError {
    StoreError::Database(Box::new(e))
}

/// Hex-encode bytes without pulling in the `hex` crate.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Persistent store backed by sled, implementing all wacore storage traits.
pub struct SledStore {
    #[allow(dead_code)]
    db: sled::Db,
    identities: sled::Tree,
    sessions: sled::Tree,
    prekeys: sled::Tree,
    signed_prekeys: sled::Tree,
    sender_keys: sled::Tree,
    sync_keys: sled::Tree,
    app_state_versions: sled::Tree,
    mutation_macs: sled::Tree,
    mutation_mac_indexes: sled::Tree,
    device_data: sled::Tree,
    device_id: AtomicI32,
    lid_mappings: sled::Tree,
    pn_mappings: sled::Tree,
    device_list_records: sled::Tree,
    sender_key_devices: sled::Tree,
    base_keys: sled::Tree,
    tc_tokens: sled::Tree,
    sent_messages: sled::Tree,
    msg_secrets: sled::Tree,
}

fn json_err(e: serde_json::Error) -> StoreError {
    StoreError::Serialization(Box::new(e))
}

fn postcard_err(e: postcard::Error) -> StoreError {
    StoreError::Serialization(Box::new(e))
}

/// Format tag prefixed to every encoded record.
const FORMAT_POSTCARD: u8 = 0x01;

/// Key prefix for all sender-key-device rows of a group (`<group>\0`).
/// `\0` cannot appear in a JID, so the prefix is unambiguous.
fn sender_key_device_prefix(group_jid: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(group_jid.len() + 1);
    p.extend_from_slice(group_jid.as_bytes());
    p.push(0);
    p
}

/// Full row key for a (group, device) pair: `<group>\0<device>`.
fn sender_key_device_key(group_jid: &str, device_jid: &str) -> Vec<u8> {
    let mut k = sender_key_device_prefix(group_jid);
    k.extend_from_slice(device_jid.as_bytes());
    k
}

fn encode_persistent<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let body = postcard::to_allocvec(value).map_err(postcard_err)?;
    let mut buf = Vec::with_capacity(1 + body.len());
    buf.push(FORMAT_POSTCARD);
    buf.extend_from_slice(&body);
    Ok(buf)
}

fn decode_persistent<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.first() == Some(&FORMAT_POSTCARD) {
        return postcard::from_bytes::<T>(&bytes[1..]).map_err(postcard_err);
    }
    // Legacy: untagged data is JSON (pre-postcard migration).
    serde_json::from_slice(bytes).map_err(json_err)
}

impl SledStore {
    /// Open or create a sled database at the given path.
    pub fn open(path: impl AsRef<Path>) -> std::result::Result<Self, sled::Error> {
        let db = sled::open(path)?;

        // Load persisted device_id counter.
        let device_id_tree = db.open_tree("device_id")?;
        let id_val = device_id_tree
            .get(b"counter")?
            .and_then(|v| v.as_ref().try_into().ok())
            .map(i32::from_le_bytes)
            .unwrap_or(0);

        Ok(Self {
            identities: db.open_tree("identities")?,
            sessions: db.open_tree("sessions")?,
            prekeys: db.open_tree("prekeys")?,
            signed_prekeys: db.open_tree("signed_prekeys")?,
            sender_keys: db.open_tree("sender_keys")?,
            sync_keys: db.open_tree("sync_keys")?,
            app_state_versions: db.open_tree("app_state_versions")?,
            mutation_macs: db.open_tree("mutation_macs")?,
            mutation_mac_indexes: db.open_tree("mutation_mac_indexes")?,
            device_data: db.open_tree("device_data")?,
            device_id: AtomicI32::new(id_val),
            lid_mappings: db.open_tree("lid_mappings")?,
            pn_mappings: db.open_tree("pn_mappings")?,
            device_list_records: db.open_tree("device_list_records")?,
            sender_key_devices: db.open_tree("sender_key_devices")?,
            base_keys: db.open_tree("base_keys")?,
            tc_tokens: db.open_tree("tc_tokens")?,
            sent_messages: db.open_tree("sent_messages")?,
            msg_secrets: db.open_tree("msg_secrets")?,
            db,
        })
    }
}

// ============================================================================
// SignalStore
// ============================================================================

#[async_trait]
impl SignalStore for SledStore {
    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        self.identities
            .insert(address.as_bytes(), &key[..])
            .map_err(db_err)?;
        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<[u8; 32]>> {
        Ok(self
            .identities
            .get(address.as_bytes())
            .map_err(db_err)?
            .and_then(|v| v.as_ref().try_into().ok()))
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        self.identities.remove(address.as_bytes()).map_err(db_err)?;
        Ok(())
    }

    async fn get_session(&self, address: &str) -> Result<Option<Bytes>> {
        Ok(self
            .sessions
            .get(address.as_bytes())
            .map_err(db_err)?
            .map(|v| Bytes::copy_from_slice(&v)))
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        self.sessions
            .insert(address.as_bytes(), session)
            .map_err(db_err)?;
        Ok(())
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        self.sessions.remove(address.as_bytes()).map_err(db_err)?;
        Ok(())
    }

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        // Store as JSON: [record_bytes, uploaded_bool]
        let val = serde_json::to_vec(&(record, uploaded)).map_err(json_err)?;
        self.prekeys
            .insert(id.to_le_bytes(), val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Bytes>> {
        match self.prekeys.get(id.to_le_bytes()).map_err(db_err)? {
            Some(v) => {
                let (record, _uploaded): (Vec<u8>, bool) =
                    serde_json::from_slice(&v).map_err(json_err)?;
                Ok(Some(Bytes::from(record)))
            },
            None => Ok(None),
        }
    }

    async fn mark_prekeys_uploaded(&self, ids: &[u32]) -> Result<()> {
        for id in ids {
            if let Some(v) = self.prekeys.get(id.to_le_bytes()).map_err(db_err)? {
                let (record, _uploaded): (Vec<u8>, bool) =
                    serde_json::from_slice(&v).map_err(json_err)?;
                let val = serde_json::to_vec(&(record, true)).map_err(json_err)?;
                self.prekeys
                    .insert(id.to_le_bytes(), val.as_slice())
                    .map_err(db_err)?;
            }
        }
        Ok(())
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        self.prekeys.remove(id.to_le_bytes()).map_err(db_err)?;
        Ok(())
    }

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        self.signed_prekeys
            .insert(id.to_le_bytes(), record)
            .map_err(db_err)?;
        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        Ok(self
            .signed_prekeys
            .get(id.to_le_bytes())
            .map_err(db_err)?
            .map(|v| v.to_vec()))
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut result = Vec::new();
        for entry in self.signed_prekeys.iter() {
            let (k, v) = entry.map_err(db_err)?;
            if let Ok(bytes) = k.as_ref().try_into() {
                let id = u32::from_le_bytes(bytes);
                result.push((id, v.to_vec()));
            }
        }
        Ok(result)
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        self.signed_prekeys
            .remove(id.to_le_bytes())
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_max_prekey_id(&self) -> Result<u32> {
        let mut max_id = 0u32;
        for entry in self.prekeys.iter() {
            let (k, _) = entry.map_err(db_err)?;
            if let Ok(bytes) = k.as_ref().try_into() {
                let id = u32::from_le_bytes(bytes);
                if id > max_id {
                    max_id = id;
                }
            }
        }
        Ok(max_id)
    }

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        self.sender_keys
            .insert(address.as_bytes(), record)
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .sender_keys
            .get(address.as_bytes())
            .map_err(db_err)?
            .map(|v| v.to_vec()))
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        self.sender_keys
            .remove(address.as_bytes())
            .map_err(db_err)?;
        Ok(())
    }
}

// ============================================================================
// AppSyncStore
// ============================================================================

#[async_trait]
impl AppSyncStore for SledStore {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        match self.sync_keys.get(key_id).map_err(db_err)? {
            Some(v) => Ok(Some(decode_persistent(&v)?)),
            None => Ok(None),
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let val = encode_persistent(&key)?;
        self.sync_keys
            .insert(key_id, val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_version(&self, name: &str) -> Result<HashState> {
        match self
            .app_state_versions
            .get(name.as_bytes())
            .map_err(db_err)?
        {
            Some(v) => decode_persistent(&v),
            None => Ok(HashState::default()),
        }
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let val = encode_persistent(&state)?;
        self.app_state_versions
            .insert(name.as_bytes(), val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn put_mutation_macs(
        &self,
        name: &str,
        version: u64,
        mutations: &[AppStateMutationMAC],
    ) -> Result<()> {
        let version_key = format!("{name}:{version}");
        let mut indexes = Vec::new();
        for mac in mutations {
            let mac_key = format!("{name}:{version}:{}", hex_encode(&mac.index_mac));
            self.mutation_macs
                .insert(mac_key.as_bytes(), mac.value_mac.as_slice())
                .map_err(db_err)?;
            indexes.push(mac.index_mac.clone());
        }
        let idx_val = encode_persistent(&indexes)?;
        self.mutation_mac_indexes
            .insert(version_key.as_bytes(), idx_val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_mutation_mac(&self, name: &str, index_mac: &[u8]) -> Result<Option<Vec<u8>>> {
        let prefix = format!("{name}:");
        let hex_mac = hex_encode(index_mac);
        for entry in self.mutation_mac_indexes.iter() {
            let (k, _) = entry.map_err(db_err)?;
            let key_str = String::from_utf8_lossy(&k);
            if key_str.starts_with(&prefix) {
                let mac_key = format!("{key_str}:{hex_mac}");
                if let Some(value_mac) =
                    self.mutation_macs.get(mac_key.as_bytes()).map_err(db_err)?
                {
                    return Ok(Some(value_mac.to_vec()));
                }
            }
        }
        Ok(None)
    }

    async fn delete_mutation_macs(&self, name: &str, index_macs: &[Vec<u8>]) -> Result<()> {
        for index_mac in index_macs {
            let hex_mac = hex_encode(index_mac);
            let prefix = format!("{name}:");
            let mut keys_to_remove = Vec::new();
            for entry in self.mutation_macs.iter() {
                let (k, _) = entry.map_err(db_err)?;
                let key_str = String::from_utf8_lossy(&k);
                if key_str.starts_with(&prefix) && key_str.ends_with(&hex_mac) {
                    keys_to_remove.push(k);
                }
            }
            for key in keys_to_remove {
                self.mutation_macs.remove(key).map_err(db_err)?;
            }
        }
        Ok(())
    }

    async fn clear_mutation_macs(&self, name: &str) -> Result<()> {
        let prefix = format!("{name}:");
        for tree in [&self.mutation_macs, &self.mutation_mac_indexes] {
            let mut keys_to_remove = Vec::new();
            for entry in tree.iter() {
                let (k, _) = entry.map_err(db_err)?;
                if String::from_utf8_lossy(&k).starts_with(&prefix) {
                    keys_to_remove.push(k);
                }
            }
            for key in keys_to_remove {
                tree.remove(key).map_err(db_err)?;
            }
        }
        Ok(())
    }

    async fn get_latest_sync_key_id(&self) -> Result<Option<Vec<u8>>> {
        Ok(self
            .sync_keys
            .last()
            .map_err(db_err)?
            .map(|(k, _)| k.to_vec()))
    }
}

// ============================================================================
// MsgSecretStore
// ============================================================================

/// Composite row key `chat\0sender\0msg_id` (`\0` cannot appear in JIDs/ids).
fn msg_secret_key(chat: &str, sender: &str, msg_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(chat.len() + sender.len() + msg_id.len() + 2);
    k.extend_from_slice(chat.as_bytes());
    k.push(0);
    k.extend_from_slice(sender.as_bytes());
    k.push(0);
    k.extend_from_slice(msg_id.as_bytes());
    k
}

#[async_trait]
impl MsgSecretStore for SledStore {
    async fn put_msg_secrets(&self, entries: Vec<MsgSecretEntry>) -> Result<usize> {
        let count = entries.len();
        for entry in entries {
            let key = msg_secret_key(&entry.chat, &entry.sender, &entry.msg_id);
            let (expires_at, message_ts) =
                match self.msg_secrets.get(key.as_slice()).map_err(db_err)? {
                    Some(v) => {
                        let (_, existing_exp, existing_ts): (Vec<u8>, i64, i64) =
                            decode_persistent(&v)?;
                        (
                            merge_msg_secret_expiry(existing_exp, entry.expires_at),
                            merge_msg_secret_message_ts(existing_ts, entry.message_ts),
                        )
                    },
                    None => (entry.expires_at, entry.message_ts),
                };
            let val = encode_persistent(&(entry.secret, expires_at, message_ts))?;
            self.msg_secrets
                .insert(key, val.as_slice())
                .map_err(db_err)?;
        }
        Ok(count)
    }

    async fn get_msg_secret(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<Vec<u8>>> {
        match self
            .msg_secrets
            .get(msg_secret_key(chat, sender, msg_id))
            .map_err(db_err)?
        {
            Some(v) => {
                let (secret, ..): (Vec<u8>, i64, i64) = decode_persistent(&v)?;
                Ok(Some(secret))
            },
            None => Ok(None),
        }
    }

    async fn get_msg_secret_with_ts(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<(Vec<u8>, i64)>> {
        match self
            .msg_secrets
            .get(msg_secret_key(chat, sender, msg_id))
            .map_err(db_err)?
        {
            Some(v) => {
                let (secret, _, message_ts): (Vec<u8>, i64, i64) = decode_persistent(&v)?;
                Ok(Some((secret, message_ts)))
            },
            None => Ok(None),
        }
    }

    async fn delete_expired_msg_secrets(&self, cutoff_timestamp: i64) -> Result<u32> {
        let mut count = 0u32;
        let mut keys_to_remove = Vec::new();
        for entry in self.msg_secrets.iter() {
            let (k, v) = entry.map_err(db_err)?;
            let (_, expires_at, _): (Vec<u8>, i64, i64) = decode_persistent(&v)?;
            if expires_at != 0 && expires_at <= cutoff_timestamp {
                keys_to_remove.push(k);
            }
        }
        for key in keys_to_remove {
            self.msg_secrets.remove(key).map_err(db_err)?;
            count += 1;
        }
        Ok(count)
    }
}

// ============================================================================
// ProtocolStore
// ============================================================================

#[async_trait]
impl ProtocolStore for SledStore {
    async fn get_sender_key_devices(&self, group_jid: &str) -> Result<Vec<(String, bool)>> {
        let prefix = sender_key_device_prefix(group_jid);
        let mut result = Vec::new();
        for entry in self.sender_key_devices.scan_prefix(&prefix) {
            let (k, v) = entry.map_err(db_err)?;
            let device = String::from_utf8_lossy(&k[prefix.len()..]).into_owned();
            result.push((device, v.first() == Some(&1u8)));
        }
        Ok(result)
    }

    async fn set_sender_key_status(&self, group_jid: &str, entries: &[(&str, bool)]) -> Result<()> {
        for (device_jid, has_key) in entries {
            let key = sender_key_device_key(group_jid, device_jid);
            self.sender_key_devices
                .insert(key, &[u8::from(*has_key)])
                .map_err(db_err)?;
        }
        Ok(())
    }

    async fn clear_sender_key_devices(&self, group_jid: &str) -> Result<()> {
        let prefix = sender_key_device_prefix(group_jid);
        let mut keys_to_remove = Vec::new();
        for entry in self.sender_key_devices.scan_prefix(&prefix) {
            let (k, _) = entry.map_err(db_err)?;
            keys_to_remove.push(k);
        }
        for key in keys_to_remove {
            self.sender_key_devices.remove(key).map_err(db_err)?;
        }
        Ok(())
    }

    async fn delete_sender_key_device_rows(&self, device_jids: &[&str]) -> Result<()> {
        let mut keys_to_remove = Vec::new();
        for entry in self.sender_key_devices.iter() {
            let (k, _) = entry.map_err(db_err)?;
            let key_str = String::from_utf8_lossy(&k);
            if let Some((_, device)) = key_str.split_once('\0')
                && device_jids.contains(&device)
            {
                keys_to_remove.push(k);
            }
        }
        for key in keys_to_remove {
            self.sender_key_devices.remove(key).map_err(db_err)?;
        }
        Ok(())
    }

    async fn clear_all_sender_key_devices(&self) -> Result<()> {
        self.sender_key_devices.clear().map_err(db_err)?;
        Ok(())
    }

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        match self.lid_mappings.get(lid.as_bytes()).map_err(db_err)? {
            Some(v) => Ok(Some(decode_persistent(&v)?)),
            None => Ok(None),
        }
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        if let Some(lid) = self.pn_mappings.get(phone.as_bytes()).map_err(db_err)? {
            let lid_str = String::from_utf8_lossy(&lid);
            return self.get_lid_mapping(&lid_str).await;
        }
        Ok(None)
    }

    async fn put_lid_mapping(&self, entry: &LidPnMappingEntry) -> Result<()> {
        self.pn_mappings
            .insert(entry.phone_number.as_bytes(), entry.lid.as_bytes())
            .map_err(db_err)?;
        let val = encode_persistent(entry)?;
        self.lid_mappings
            .insert(entry.lid.as_bytes(), val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        let mut result = Vec::new();
        for entry in self.lid_mappings.iter() {
            let (_, v) = entry.map_err(db_err)?;
            let mapping: LidPnMappingEntry = decode_persistent(&v)?;
            result.push(mapping);
        }
        Ok(result)
    }

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        let key = format!("{address}:{message_id}");
        self.base_keys
            .insert(key.as_bytes(), base_key)
            .map_err(db_err)?;
        Ok(())
    }

    async fn has_same_base_key(
        &self,
        address: &str,
        message_id: &str,
        current_base_key: &[u8],
    ) -> Result<bool> {
        let key = format!("{address}:{message_id}");
        Ok(self
            .base_keys
            .get(key.as_bytes())
            .map_err(db_err)?
            .is_some_and(|v| v.as_ref() == current_base_key))
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        let key = format!("{address}:{message_id}");
        self.base_keys.remove(key.as_bytes()).map_err(db_err)?;
        Ok(())
    }

    async fn update_device_list(&self, record: DeviceListRecord) -> Result<()> {
        // JSON, not postcard: `DeviceListRecord.raw_id` is marked
        // `skip_serializing_if`, which postcard (non-self-describing) cannot
        // round-trip. JSON also tolerates future field additions.
        let val = serde_json::to_vec(&record).map_err(json_err)?;
        self.device_list_records
            .insert(record.user.as_bytes(), val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_devices(&self, user: &str) -> Result<Option<DeviceListRecord>> {
        match self
            .device_list_records
            .get(user.as_bytes())
            .map_err(db_err)?
        {
            // Device lists are a re-fetchable usync cache: records persisted
            // by older releases (pre-`raw_id` postcard layout) no longer
            // decode, so treat them as missing and let usync repopulate.
            Some(v) => match decode_persistent(&v) {
                Ok(record) => Ok(Some(record)),
                Err(e) => {
                    debug!(
                        user,
                        error = %e,
                        "evicting undecodable device-list record; usync repopulates it"
                    );
                    self.device_list_records
                        .remove(user.as_bytes())
                        .map_err(db_err)?;
                    Ok(None)
                },
            },
            None => Ok(None),
        }
    }

    async fn delete_devices(&self, user: &str) -> Result<()> {
        self.device_list_records
            .remove(user.as_bytes())
            .map_err(db_err)?;
        Ok(())
    }

    // --- TcToken Storage ---

    async fn get_tc_token(&self, jid: &str) -> Result<Option<TcTokenEntry>> {
        match self.tc_tokens.get(jid.as_bytes()).map_err(db_err)? {
            Some(v) => Ok(Some(decode_persistent(&v)?)),
            None => Ok(None),
        }
    }

    async fn put_tc_token(&self, jid: &str, entry: &TcTokenEntry) -> Result<()> {
        let val = encode_persistent(entry)?;
        self.tc_tokens
            .insert(jid.as_bytes(), val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn delete_tc_token(&self, jid: &str) -> Result<()> {
        self.tc_tokens.remove(jid.as_bytes()).map_err(db_err)?;
        Ok(())
    }

    async fn get_all_tc_token_jids(&self) -> Result<Vec<String>> {
        let mut jids = Vec::new();
        for entry in self.tc_tokens.iter() {
            let (k, _) = entry.map_err(db_err)?;
            jids.push(String::from_utf8_lossy(&k).into_owned());
        }
        Ok(jids)
    }

    async fn delete_expired_tc_tokens(&self, cutoff_timestamp: i64) -> Result<u32> {
        let mut count = 0u32;
        let mut keys_to_remove = Vec::new();
        for entry in self.tc_tokens.iter() {
            let (k, v) = entry.map_err(db_err)?;
            let token: TcTokenEntry = decode_persistent(&v)?;
            if token.token_timestamp < cutoff_timestamp {
                keys_to_remove.push(k);
            }
        }
        for key in keys_to_remove {
            self.tc_tokens.remove(key).map_err(db_err)?;
            count += 1;
        }
        Ok(count)
    }

    // --- Sent Message Store ---

    async fn store_sent_message(
        &self,
        chat_jid: &str,
        message_id: &str,
        payload: &[u8],
    ) -> Result<()> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let key = format!("{chat_jid}:{message_id}");
        let val = encode_persistent(&(payload.to_vec(), now))?;
        self.sent_messages
            .insert(key.as_bytes(), val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn take_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        let key = format!("{chat_jid}:{message_id}");
        match self.sent_messages.remove(key.as_bytes()).map_err(db_err)? {
            Some(v) => {
                let (payload, _ts): (Vec<u8>, i64) = decode_persistent(&v)?;
                Ok(Some(payload))
            },
            None => Ok(None),
        }
    }

    async fn delete_expired_sent_messages(&self, cutoff_timestamp: i64) -> Result<u32> {
        let mut count = 0u32;
        let mut keys_to_remove = Vec::new();
        for entry in self.sent_messages.iter() {
            let (k, v) = entry.map_err(db_err)?;
            let (_payload, ts): (Vec<u8>, i64) = decode_persistent(&v)?;
            if ts < cutoff_timestamp {
                keys_to_remove.push(k);
            }
        }
        for key in keys_to_remove {
            self.sent_messages.remove(key).map_err(db_err)?;
            count += 1;
        }
        Ok(count)
    }
}

// ============================================================================
// DeviceStore
// ============================================================================

/// Persisted layout of `wacore::store::Device` as of whatsapp-rust 0.5.
///
/// postcard is not self-describing, so records written before 0.6 fail to
/// decode once `Device` gained trailing fields (`server_has_prekeys`,
/// `nct_salt`, `server_cert_chain`). Losing this record would drop the
/// WhatsApp pairing and force a QR re-scan, so `load()` falls back to this
/// shim and upgrades the record in place. Field order, types and serde
/// attributes must match 0.5 exactly.
#[derive(serde::Serialize, serde::Deserialize)]
struct LegacyDevice05 {
    pn: Option<wacore_binary::jid::Jid>,
    lid: Option<wacore_binary::jid::Jid>,
    registration_id: u32,
    #[serde(with = "wacore::store::device::key_pair_serde")]
    noise_key: wacore::libsignal::protocol::KeyPair,
    #[serde(with = "wacore::store::device::key_pair_serde")]
    identity_key: wacore::libsignal::protocol::KeyPair,
    #[serde(with = "wacore::store::device::key_pair_serde")]
    signed_pre_key: wacore::libsignal::protocol::KeyPair,
    signed_pre_key_id: u32,
    #[serde(with = "serde_big_array::BigArray")]
    signed_pre_key_signature: [u8; 64],
    adv_secret_key: [u8; 32],
    #[serde(with = "wacore::store::device::account_serde", default)]
    account: Option<std::sync::Arc<waproto::whatsapp::AdvSignedDeviceIdentity>>,
    push_name: String,
    app_version_primary: u32,
    app_version_secondary: u32,
    app_version_tertiary: u32,
    app_version_last_fetched_ms: i64,
    #[serde(default)]
    edge_routing_info: Option<Vec<u8>>,
    #[serde(default)]
    props_hash: Option<String>,
    #[serde(default)]
    next_pre_key_id: u32,
}

impl From<LegacyDevice05> for wacore::store::Device {
    fn from(legacy: LegacyDevice05) -> Self {
        Self {
            pn: legacy.pn,
            lid: legacy.lid,
            registration_id: legacy.registration_id,
            noise_key: legacy.noise_key,
            identity_key: legacy.identity_key,
            signed_pre_key: legacy.signed_pre_key,
            signed_pre_key_id: legacy.signed_pre_key_id,
            signed_pre_key_signature: legacy.signed_pre_key_signature,
            adv_secret_key: legacy.adv_secret_key,
            account: legacy.account,
            push_name: legacy.push_name,
            app_version_primary: legacy.app_version_primary,
            app_version_secondary: legacy.app_version_secondary,
            app_version_tertiary: legacy.app_version_tertiary,
            app_version_last_fetched_ms: legacy.app_version_last_fetched_ms,
            device_props: Default::default(),
            client_profile: Default::default(),
            edge_routing_info: legacy.edge_routing_info,
            props_hash: legacy.props_hash,
            next_pre_key_id: legacy.next_pre_key_id,
            first_unupload_pre_key_id: 0,
            server_has_prekeys: false,
            nct_salt: None,
            nct_salt_sync_seen: false,
            server_cert_chain: None,
            login_counter: 0,
            // 0.5-era records predate the migration flag. `false` is the safe
            // default: PN wire addressing delivers for both populations, and a
            // migrated account re-learns the flag from the primary's mapping
            // sync on the next connect.
            lid_migrated: false,
        }
    }
}

#[async_trait]
impl DeviceStore for SledStore {
    async fn save(&self, device: &wacore::store::Device) -> Result<()> {
        let val = encode_persistent(device)?;
        self.device_data
            .insert(b"device", val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn load(&self) -> Result<Option<wacore::store::Device>> {
        let Some(v) = self.device_data.get(b"device").map_err(db_err)? else {
            return Ok(None);
        };
        match decode_persistent::<wacore::store::Device>(&v) {
            Ok(device) => Ok(Some(device)),
            Err(primary) => {
                // Pre-0.6 record: decode with the legacy shim and upgrade the
                // stored bytes so subsequent loads use the current layout.
                let legacy = decode_persistent::<LegacyDevice05>(&v).map_err(|fallback| {
                    warn!(
                        primary = %primary,
                        fallback = %fallback,
                        "device record failed both the current and the legacy 0.5 decode"
                    );
                    fallback
                })?;
                let device: wacore::store::Device = legacy.into();
                self.save(&device).await?;
                info!("migrated a pre-0.6 device record to the current layout");
                Ok(Some(device))
            },
        }
    }

    async fn exists(&self) -> Result<bool> {
        Ok(self.device_data.get(b"device").map_err(db_err)?.is_some())
    }

    async fn create(&self) -> Result<i32> {
        let id = self
            .device_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Persist the counter.
        let tree = self.db.open_tree("device_id").map_err(db_err)?;
        tree.insert(b"counter", &(id + 1).to_le_bytes())
            .map_err(db_err)?;
        Ok(id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp_store() -> SledStore {
        let dir = tempfile::tempdir().unwrap();
        SledStore::open(dir.path()).unwrap()
    }

    fn close_store(store: SledStore) {
        store.db.flush().unwrap();
        drop(store);
    }

    #[tokio::test]
    async fn identity_roundtrip() {
        let store = temp_store();
        let key = [42u8; 32];
        store
            .put_identity("test@s.whatsapp.net", key)
            .await
            .unwrap();
        let loaded = store.load_identity("test@s.whatsapp.net").await.unwrap();
        assert_eq!(loaded, Some(key));

        store.delete_identity("test@s.whatsapp.net").await.unwrap();
        assert!(
            store
                .load_identity("test@s.whatsapp.net")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn session_roundtrip() {
        let store = temp_store();
        let data = b"session-data";
        store.put_session("addr", data).await.unwrap();
        let loaded = store.get_session("addr").await.unwrap();
        assert_eq!(loaded, Some(Bytes::from_static(data)));
        assert!(store.has_session("addr").await.unwrap());
        assert!(!store.has_session("missing").await.unwrap());
    }

    #[tokio::test]
    async fn device_store_roundtrip() {
        let store = temp_store();
        assert!(!store.exists().await.unwrap());
        let id = store.create().await.unwrap();
        assert_eq!(id, 0);
        let id2 = store.create().await.unwrap();
        assert_eq!(id2, 1);
    }

    #[tokio::test]
    async fn prekey_operations() {
        let store = temp_store();
        store.store_prekey(1, b"pk1", false).await.unwrap();
        store.store_prekey(2, b"pk2", true).await.unwrap();
        assert_eq!(
            store.load_prekey(1).await.unwrap(),
            Some(Bytes::from_static(b"pk1"))
        );
        store.remove_prekey(1).await.unwrap();
        assert!(store.load_prekey(1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn signed_prekey_operations() {
        let store = temp_store();
        store.store_signed_prekey(10, b"spk10").await.unwrap();
        store.store_signed_prekey(20, b"spk20").await.unwrap();
        let all = store.load_all_signed_prekeys().await.unwrap();
        assert_eq!(all.len(), 2);
        store.remove_signed_prekey(10).await.unwrap();
        let all = store.load_all_signed_prekeys().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn sender_key_roundtrip() {
        let store = temp_store();
        store.put_sender_key("addr1", b"key1").await.unwrap();
        assert_eq!(
            store.get_sender_key("addr1").await.unwrap(),
            Some(b"key1".to_vec())
        );
        store.delete_sender_key("addr1").await.unwrap();
        assert!(store.get_sender_key("addr1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn sync_key_roundtrip() {
        let store = temp_store();
        let key = AppStateSyncKey {
            key_data: vec![1, 2, 3],
            fingerprint: vec![4, 5],
            timestamp: 12345,
        };
        store.set_sync_key(b"test-key", key.clone()).await.unwrap();
        let loaded = store.get_sync_key(b"test-key").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().timestamp, 12345);
    }

    #[tokio::test]
    async fn version_roundtrip() {
        let store = temp_store();
        let state = store.get_version("contacts").await.unwrap();
        assert_eq!(state.version, 0);

        let new_state = HashState {
            version: 5,
            ..Default::default()
        };
        store.set_version("contacts", new_state).await.unwrap();
        let loaded = store.get_version("contacts").await.unwrap();
        assert_eq!(loaded.version, 5);
    }

    #[tokio::test]
    async fn app_state_persistence_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();

        {
            let store = SledStore::open(dir.path()).unwrap();
            let key = AppStateSyncKey {
                key_data: vec![10, 20, 30],
                fingerprint: vec![40, 50],
                timestamp: 98765,
            };
            store.set_sync_key(b"persist-key", key).await.unwrap();
            store
                .set_version("regular_high", HashState {
                    version: 9,
                    ..Default::default()
                })
                .await
                .unwrap();
            close_store(store);
        }

        {
            let store = SledStore::open(dir.path()).unwrap();
            let loaded_key = store.get_sync_key(b"persist-key").await.unwrap();
            assert!(loaded_key.is_some());
            assert_eq!(loaded_key.unwrap().timestamp, 98765);

            let loaded_state = store.get_version("regular_high").await.unwrap();
            assert_eq!(loaded_state.version, 9);
        }
    }

    #[tokio::test]
    async fn sender_key_devices() {
        let store = temp_store();
        assert!(
            store
                .get_sender_key_devices("group1@g.us")
                .await
                .unwrap()
                .is_empty()
        );

        store
            .set_sender_key_status("group1@g.us", &[
                ("dev1:1@s.whatsapp.net", true),
                ("dev2:2@s.whatsapp.net", false),
            ])
            .await
            .unwrap();
        // Same device in another group must not leak into group1 queries.
        store
            .set_sender_key_status("group2@g.us", &[("dev1:1@s.whatsapp.net", true)])
            .await
            .unwrap();

        let mut devices = store.get_sender_key_devices("group1@g.us").await.unwrap();
        devices.sort();
        assert_eq!(devices, vec![
            ("dev1:1@s.whatsapp.net".to_string(), true),
            ("dev2:2@s.whatsapp.net".to_string(), false),
        ]);

        // Delete one device's rows across all groups.
        store
            .delete_sender_key_device_rows(&["dev1:1@s.whatsapp.net"])
            .await
            .unwrap();
        assert_eq!(
            store
                .get_sender_key_devices("group1@g.us")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .get_sender_key_devices("group2@g.us")
                .await
                .unwrap()
                .is_empty()
        );

        store.clear_sender_key_devices("group1@g.us").await.unwrap();
        assert!(
            store
                .get_sender_key_devices("group1@g.us")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn clear_all_sender_key_devices_wipes_every_group() {
        let store = temp_store();
        store
            .set_sender_key_status("group1@g.us", &[("dev1:1@s.whatsapp.net", true)])
            .await
            .unwrap();
        store
            .set_sender_key_status("group2@g.us", &[("dev2:2@s.whatsapp.net", true)])
            .await
            .unwrap();
        store.clear_all_sender_key_devices().await.unwrap();
        assert!(
            store
                .get_sender_key_devices("group1@g.us")
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .get_sender_key_devices("group2@g.us")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn lid_mapping() {
        let store = temp_store();
        let entry = LidPnMappingEntry {
            lid: "100000012345678".into(),
            phone_number: "559980000001".into(),
            created_at: 1000,
            updated_at: 2000,
            learning_source: "usync".into(),
        };
        store.put_lid_mapping(&entry).await.unwrap();

        let by_lid = store.get_lid_mapping("100000012345678").await.unwrap();
        assert!(by_lid.is_some());
        assert_eq!(by_lid.unwrap().phone_number, "559980000001");

        let by_pn = store.get_pn_mapping("559980000001").await.unwrap();
        assert!(by_pn.is_some());

        let all = store.get_all_lid_mappings().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn base_key_operations() {
        let store = temp_store();
        let key = b"base-key-data";
        store.save_base_key("addr", "msg1", key).await.unwrap();
        assert!(store.has_same_base_key("addr", "msg1", key).await.unwrap());
        assert!(
            !store
                .has_same_base_key("addr", "msg1", b"other")
                .await
                .unwrap()
        );
        store.delete_base_key("addr", "msg1").await.unwrap();
        assert!(!store.has_same_base_key("addr", "msg1", key).await.unwrap());
    }

    #[tokio::test]
    async fn device_list() {
        let store = temp_store();
        let record = DeviceListRecord {
            user: "user1".into(),
            devices: vec![DeviceInfo {
                device_id: 0,
                key_index: Some(1),
            }],
            timestamp: 1000,
            phash: None,
            raw_id: None,
        };
        store.update_device_list(record).await.unwrap();
        let loaded = store.get_devices("user1").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().devices.len(), 1);
    }

    #[tokio::test]
    async fn device_list_persistence_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();

        {
            let store = SledStore::open(dir.path()).unwrap();
            store
                .update_device_list(DeviceListRecord {
                    user: "persist-user".into(),
                    devices: vec![DeviceInfo {
                        device_id: 7,
                        key_index: Some(2),
                    }],
                    timestamp: 1234,
                    phash: None,
                    raw_id: None,
                })
                .await
                .unwrap();
            close_store(store);
        }

        {
            let store = SledStore::open(dir.path()).unwrap();
            let loaded = store.get_devices("persist-user").await.unwrap();
            assert!(loaded.is_some());
            let loaded = loaded.unwrap();
            assert_eq!(loaded.devices.len(), 1);
            assert_eq!(loaded.devices[0].device_id, 7);
            assert_eq!(loaded.timestamp, 1234);
        }
    }

    #[tokio::test]
    async fn legacy_device_record_migrates_on_load() {
        let store = temp_store();
        let mut rng = rand::rng();

        // Simulate a device record persisted by the whatsapp-rust 0.5 build:
        // same leading fields, no 0.6 trailing fields.
        let legacy = LegacyDevice05 {
            pn: Some(wacore_binary::jid::Jid::pn("15551234567")),
            lid: None,
            registration_id: 42,
            noise_key: wacore::libsignal::protocol::KeyPair::generate(&mut rng),
            identity_key: wacore::libsignal::protocol::KeyPair::generate(&mut rng),
            signed_pre_key: wacore::libsignal::protocol::KeyPair::generate(&mut rng),
            signed_pre_key_id: 1,
            signed_pre_key_signature: [7u8; 64],
            adv_secret_key: [9u8; 32],
            account: None,
            push_name: "Moltis".into(),
            app_version_primary: 2,
            app_version_secondary: 3000,
            app_version_tertiary: 1,
            app_version_last_fetched_ms: 1000,
            edge_routing_info: None,
            props_hash: None,
            next_pre_key_id: 5,
        };
        let bytes = encode_persistent(&legacy).unwrap();
        store
            .device_data
            .insert(b"device", bytes.as_slice())
            .unwrap();

        // Load must fall back to the legacy shim, not lose the pairing.
        let device = store.load().await.unwrap().unwrap();
        assert_eq!(device.registration_id, 42);
        assert_eq!(device.push_name, "Moltis");
        assert_eq!(device.next_pre_key_id, 5);
        assert_eq!(device.signed_pre_key_signature, [7u8; 64]);
        assert!(!device.server_has_prekeys);
        assert!(device.server_cert_chain.is_none());

        // The record is upgraded in place: it now decodes as the current
        // format without the fallback.
        let raw = store.device_data.get(b"device").unwrap().unwrap();
        decode_persistent::<wacore::store::Device>(&raw).unwrap();
    }

    #[tokio::test]
    async fn persistence_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();

        // Write some data.
        {
            let store = SledStore::open(dir.path()).unwrap();
            store
                .put_identity("test@s.whatsapp.net", [1u8; 32])
                .await
                .unwrap();
            store.put_session("addr", b"session-data").await.unwrap();
            let id = store.create().await.unwrap();
            assert_eq!(id, 0);
            close_store(store);
        }

        // Reopen and verify.
        {
            let store = SledStore::open(dir.path()).unwrap();
            let identity = store.load_identity("test@s.whatsapp.net").await.unwrap();
            assert_eq!(identity, Some([1u8; 32]));
            let session = store.get_session("addr").await.unwrap();
            assert_eq!(session, Some(Bytes::from_static(b"session-data")));
            let id = store.create().await.unwrap();
            assert_eq!(id, 1); // counter persisted
        }
    }

    #[tokio::test]
    async fn max_prekey_id() {
        let store = temp_store();
        assert_eq!(store.get_max_prekey_id().await.unwrap(), 0);
        store.store_prekey(5, b"pk5", false).await.unwrap();
        store.store_prekey(10, b"pk10", true).await.unwrap();
        store.store_prekey(3, b"pk3", false).await.unwrap();
        assert_eq!(store.get_max_prekey_id().await.unwrap(), 10);
    }

    #[tokio::test]
    async fn latest_sync_key_id() {
        let store = temp_store();
        assert!(store.get_latest_sync_key_id().await.unwrap().is_none());
        let key = AppStateSyncKey {
            key_data: vec![1],
            fingerprint: vec![],
            timestamp: 1,
        };
        store.set_sync_key(b"key-1", key.clone()).await.unwrap();
        store.set_sync_key(b"key-2", key).await.unwrap();
        let latest = store.get_latest_sync_key_id().await.unwrap();
        assert!(latest.is_some());
    }

    #[tokio::test]
    async fn tc_token_roundtrip() {
        let store = temp_store();
        assert!(store.get_tc_token("user@lid").await.unwrap().is_none());

        let entry = TcTokenEntry {
            token: vec![1, 2, 3],
            token_timestamp: 1000,
            sender_timestamp: Some(900),
        };
        store.put_tc_token("user@lid", &entry).await.unwrap();
        let loaded = store.get_tc_token("user@lid").await.unwrap().unwrap();
        assert_eq!(loaded.token, vec![1, 2, 3]);
        assert_eq!(loaded.token_timestamp, 1000);

        let jids = store.get_all_tc_token_jids().await.unwrap();
        assert_eq!(jids.len(), 1);

        store.delete_tc_token("user@lid").await.unwrap();
        assert!(store.get_tc_token("user@lid").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn tc_token_expiry() {
        let store = temp_store();
        store
            .put_tc_token("old@lid", &TcTokenEntry {
                token: vec![1],
                token_timestamp: 100,
                sender_timestamp: None,
            })
            .await
            .unwrap();
        store
            .put_tc_token("new@lid", &TcTokenEntry {
                token: vec![2],
                token_timestamp: 2000,
                sender_timestamp: None,
            })
            .await
            .unwrap();

        let deleted = store.delete_expired_tc_tokens(500).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(store.get_tc_token("old@lid").await.unwrap().is_none());
        assert!(store.get_tc_token("new@lid").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn sent_message_store_and_take() {
        let store = temp_store();
        store
            .store_sent_message("chat@jid", "msg1", b"payload1")
            .await
            .unwrap();

        let taken = store.take_sent_message("chat@jid", "msg1").await.unwrap();
        assert_eq!(taken, Some(b"payload1".to_vec()));

        // Take again returns None (consumed).
        assert!(
            store
                .take_sent_message("chat@jid", "msg1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn sent_message_expiry() {
        let store = temp_store();
        store
            .store_sent_message("chat@jid", "old", b"old-payload")
            .await
            .unwrap();

        // Expire anything before far-future timestamp.
        let deleted = store.delete_expired_sent_messages(i64::MAX).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(
            store
                .take_sent_message("chat@jid", "old")
                .await
                .unwrap()
                .is_none()
        );
    }
}
