//! In-memory storage backend for whatsapp-rust.
//!
//! This is a temporary solution while `whatsapp-rust-sqlite-storage` has a
//! `libsqlite3-sys` version conflict with `sqlx`. Session state does NOT
//! persist across restarts — the user must re-scan the QR code.
//!
//! TODO: Replace with `whatsapp-rust-sqlite-storage` once sqlx 0.9 stabilises
//! (it uses a range-based libsqlite3-sys dep that resolves the conflict).

use std::{fmt::Write, sync::Arc};

use {
    async_trait::async_trait,
    bytes::Bytes,
    dashmap::DashMap,
    wacore::{
        appstate::{hash::HashState, processor::AppStateMutationMAC},
        store::{error::Result, traits::*},
    },
};

/// Hex-encode bytes without pulling in the `hex` crate.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// In-memory store implementing all wacore storage traits.
#[derive(Clone, Default)]
pub struct MemoryStore {
    identities: Arc<DashMap<String, Vec<u8>>>,
    sessions: Arc<DashMap<String, Vec<u8>>>,
    prekeys: Arc<DashMap<u32, (Vec<u8>, bool)>>,
    signed_prekeys: Arc<DashMap<u32, Vec<u8>>>,
    sender_keys: Arc<DashMap<String, Vec<u8>>>,
    sync_keys: Arc<DashMap<Vec<u8>, AppStateSyncKey>>,
    /// Most recently stored sync key ID (DashMap iteration order is non-deterministic).
    latest_sync_key_id: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    app_state_versions: Arc<DashMap<String, HashState>>,
    /// Keyed by `"{name}:{version}:{hex(index_mac)}"`.
    mutation_macs: Arc<DashMap<String, Vec<u8>>>,
    /// Keyed by `"{name}:{version}"` → list of index_macs stored at that version.
    mutation_mac_indexes: Arc<DashMap<String, Vec<Vec<u8>>>>,
    device_data: Arc<tokio::sync::RwLock<Option<wacore::store::Device>>>,
    device_id: Arc<std::sync::atomic::AtomicI32>,
    lid_mappings: Arc<DashMap<String, LidPnMappingEntry>>,
    /// Phone number → LID reverse index.
    pn_mappings: Arc<DashMap<String, String>>,
    device_list_records: Arc<DashMap<String, DeviceListRecord>>,
    /// Sender-key distribution status keyed by `(group_jid, device_jid)`.
    sender_key_devices: Arc<DashMap<(String, String), bool>>,
    /// Base keys keyed by `"{address}:{message_id}"`.
    base_keys: Arc<DashMap<String, Vec<u8>>>,
    /// TC tokens keyed by JID string.
    tc_tokens: Arc<DashMap<String, TcTokenEntry>>,
    /// Sent messages keyed by `"{chat_jid}:{message_id}"` → (payload, timestamp).
    sent_messages: Arc<DashMap<String, (Vec<u8>, i64)>>,
    /// Message secrets keyed by (chat, sender, msg_id) → (secret, expires_at, message_ts).
    msg_secrets: Arc<DashMap<(String, String, String), (Vec<u8>, i64, i64)>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

// ============================================================================
// SignalStore
// ============================================================================

#[async_trait]
impl SignalStore for MemoryStore {
    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        self.identities.insert(address.to_string(), key.to_vec());
        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<[u8; 32]>> {
        Ok(self
            .identities
            .get(address)
            .and_then(|v| v.value().as_slice().try_into().ok()))
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        self.identities.remove(address);
        Ok(())
    }

    async fn get_session(&self, address: &str) -> Result<Option<Bytes>> {
        Ok(self
            .sessions
            .get(address)
            .map(|v| Bytes::from(v.value().clone())))
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        self.sessions.insert(address.to_string(), session.to_vec());
        Ok(())
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        self.sessions.remove(address);
        Ok(())
    }

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        self.prekeys.insert(id, (record.to_vec(), uploaded));
        Ok(())
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Bytes>> {
        Ok(self
            .prekeys
            .get(&id)
            .map(|v| Bytes::from(v.value().0.clone())))
    }

    async fn mark_prekeys_uploaded(&self, ids: &[u32]) -> Result<()> {
        for id in ids {
            if let Some(mut entry) = self.prekeys.get_mut(id) {
                entry.value_mut().1 = true;
            }
        }
        Ok(())
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        self.prekeys.remove(&id);
        Ok(())
    }

    async fn get_max_prekey_id(&self) -> Result<u32> {
        Ok(self.prekeys.iter().map(|e| *e.key()).max().unwrap_or(0))
    }

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        self.signed_prekeys.insert(id, record.to_vec());
        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        Ok(self.signed_prekeys.get(&id).map(|v| v.value().clone()))
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        Ok(self
            .signed_prekeys
            .iter()
            .map(|e| (*e.key(), e.value().clone()))
            .collect())
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        self.signed_prekeys.remove(&id);
        Ok(())
    }

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        self.sender_keys
            .insert(address.to_string(), record.to_vec());
        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.sender_keys.get(address).map(|v| v.value().clone()))
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        self.sender_keys.remove(address);
        Ok(())
    }
}

// ============================================================================
// AppSyncStore
// ============================================================================

#[async_trait]
impl AppSyncStore for MemoryStore {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        Ok(self.sync_keys.get(key_id).map(|v| v.value().clone()))
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        self.sync_keys.insert(key_id.to_vec(), key);
        *self
            .latest_sync_key_id
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(key_id.to_vec());
        Ok(())
    }

    async fn get_version(&self, name: &str) -> Result<HashState> {
        Ok(self
            .app_state_versions
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default())
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        self.app_state_versions.insert(name.to_string(), state);
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
            self.mutation_macs.insert(mac_key, mac.value_mac.clone());
            indexes.push(mac.index_mac.clone());
        }
        self.mutation_mac_indexes.insert(version_key, indexes);
        Ok(())
    }

    async fn get_mutation_mac(&self, name: &str, index_mac: &[u8]) -> Result<Option<Vec<u8>>> {
        // Search across all versions for this name + index_mac combo.
        for entry in self.mutation_mac_indexes.iter() {
            if entry.key().starts_with(&format!("{name}:")) {
                let version_key = entry.key();
                let mac_key = format!("{version_key}:{}", hex_encode(index_mac));
                if let Some(value_mac) = self.mutation_macs.get(&mac_key) {
                    return Ok(Some(value_mac.value().clone()));
                }
            }
        }
        Ok(None)
    }

    async fn delete_mutation_macs(&self, name: &str, index_macs: &[Vec<u8>]) -> Result<()> {
        for index_mac in index_macs {
            let hex_mac = hex_encode(index_mac);
            // Remove from all versions.
            let keys_to_remove: Vec<String> = self
                .mutation_macs
                .iter()
                .filter(|e| e.key().starts_with(&format!("{name}:")) && e.key().ends_with(&hex_mac))
                .map(|e| e.key().clone())
                .collect();
            for key in keys_to_remove {
                self.mutation_macs.remove(&key);
            }
        }
        Ok(())
    }

    async fn clear_mutation_macs(&self, name: &str) -> Result<()> {
        let prefix = format!("{name}:");
        self.mutation_macs.retain(|k, _| !k.starts_with(&prefix));
        self.mutation_mac_indexes
            .retain(|k, _| !k.starts_with(&prefix));
        Ok(())
    }

    async fn get_latest_sync_key_id(&self) -> Result<Option<Vec<u8>>> {
        Ok(self
            .latest_sync_key_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone())
    }
}

// ============================================================================
// MsgSecretStore
// ============================================================================

#[async_trait]
impl MsgSecretStore for MemoryStore {
    async fn put_msg_secrets(&self, entries: Vec<MsgSecretEntry>) -> Result<usize> {
        let count = entries.len();
        for entry in entries {
            let key = (entry.chat, entry.sender, entry.msg_id);
            match self.msg_secrets.get_mut(&key) {
                Some(mut existing) => {
                    let (_, exp, ts) = existing.value_mut();
                    *exp = merge_msg_secret_expiry(*exp, entry.expires_at);
                    *ts = merge_msg_secret_message_ts(*ts, entry.message_ts);
                },
                None => {
                    self.msg_secrets
                        .insert(key, (entry.secret, entry.expires_at, entry.message_ts));
                },
            }
        }
        Ok(count)
    }

    async fn get_msg_secret(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .msg_secrets
            .get(&(chat.to_string(), sender.to_string(), msg_id.to_string()))
            .map(|v| v.value().0.clone()))
    }

    async fn get_msg_secret_with_ts(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<(Vec<u8>, i64)>> {
        Ok(self
            .msg_secrets
            .get(&(chat.to_string(), sender.to_string(), msg_id.to_string()))
            .map(|v| (v.value().0.clone(), v.value().2)))
    }

    async fn delete_expired_msg_secrets(&self, cutoff_timestamp: i64) -> Result<u32> {
        let before = self.msg_secrets.len();
        self.msg_secrets
            .retain(|_, (_, expires_at, _)| *expires_at == 0 || *expires_at > cutoff_timestamp);
        Ok((before - self.msg_secrets.len()) as u32)
    }
}

// ============================================================================
// ProtocolStore
// ============================================================================

#[async_trait]
impl ProtocolStore for MemoryStore {
    async fn get_sender_key_devices(&self, group_jid: &str) -> Result<Vec<(String, bool)>> {
        Ok(self
            .sender_key_devices
            .iter()
            .filter(|e| e.key().0 == group_jid)
            .map(|e| (e.key().1.clone(), *e.value()))
            .collect())
    }

    async fn set_sender_key_status(&self, group_jid: &str, entries: &[(&str, bool)]) -> Result<()> {
        for (device_jid, has_key) in entries {
            self.sender_key_devices
                .insert((group_jid.to_string(), (*device_jid).to_string()), *has_key);
        }
        Ok(())
    }

    async fn clear_sender_key_devices(&self, group_jid: &str) -> Result<()> {
        self.sender_key_devices.retain(|k, _| k.0 != group_jid);
        Ok(())
    }

    async fn delete_sender_key_device_rows(&self, device_jids: &[&str]) -> Result<()> {
        self.sender_key_devices
            .retain(|k, _| !device_jids.contains(&k.1.as_str()));
        Ok(())
    }

    async fn clear_all_sender_key_devices(&self) -> Result<()> {
        self.sender_key_devices.clear();
        Ok(())
    }

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        Ok(self.lid_mappings.get(lid).map(|v| v.value().clone()))
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        if let Some(lid) = self.pn_mappings.get(phone) {
            return Ok(self
                .lid_mappings
                .get(lid.value())
                .map(|v| v.value().clone()));
        }
        Ok(None)
    }

    async fn put_lid_mapping(&self, entry: &LidPnMappingEntry) -> Result<()> {
        self.pn_mappings
            .insert(entry.phone_number.clone(), entry.lid.clone());
        self.lid_mappings.insert(entry.lid.clone(), entry.clone());
        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        Ok(self
            .lid_mappings
            .iter()
            .map(|e| e.value().clone())
            .collect())
    }

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        let key = format!("{address}:{message_id}");
        self.base_keys.insert(key, base_key.to_vec());
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
            .get(&key)
            .is_some_and(|v| v.value() == current_base_key))
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        let key = format!("{address}:{message_id}");
        self.base_keys.remove(&key);
        Ok(())
    }

    async fn update_device_list(&self, record: DeviceListRecord) -> Result<()> {
        self.device_list_records.insert(record.user.clone(), record);
        Ok(())
    }

    async fn get_devices(&self, user: &str) -> Result<Option<DeviceListRecord>> {
        Ok(self
            .device_list_records
            .get(user)
            .map(|v| v.value().clone()))
    }

    async fn delete_devices(&self, user: &str) -> Result<()> {
        self.device_list_records.remove(user);
        Ok(())
    }

    // --- TcToken Storage ---

    async fn get_tc_token(&self, jid: &str) -> Result<Option<TcTokenEntry>> {
        Ok(self.tc_tokens.get(jid).map(|v| v.value().clone()))
    }

    async fn put_tc_token(&self, jid: &str, entry: &TcTokenEntry) -> Result<()> {
        self.tc_tokens.insert(jid.to_string(), entry.clone());
        Ok(())
    }

    async fn delete_tc_token(&self, jid: &str) -> Result<()> {
        self.tc_tokens.remove(jid);
        Ok(())
    }

    async fn get_all_tc_token_jids(&self) -> Result<Vec<String>> {
        Ok(self.tc_tokens.iter().map(|e| e.key().clone()).collect())
    }

    async fn delete_expired_tc_tokens(&self, cutoff_timestamp: i64) -> Result<u32> {
        let keys_to_remove: Vec<String> = self
            .tc_tokens
            .iter()
            .filter(|e| e.value().token_timestamp < cutoff_timestamp)
            .map(|e| e.key().clone())
            .collect();
        let count = keys_to_remove.len() as u32;
        for key in keys_to_remove {
            self.tc_tokens.remove(&key);
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
        self.sent_messages.insert(key, (payload.to_vec(), now));
        Ok(())
    }

    async fn take_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        let key = format!("{chat_jid}:{message_id}");
        Ok(self
            .sent_messages
            .remove(&key)
            .map(|(_, (payload, _))| payload))
    }

    async fn delete_expired_sent_messages(&self, cutoff_timestamp: i64) -> Result<u32> {
        let keys_to_remove: Vec<String> = self
            .sent_messages
            .iter()
            .filter(|e| e.value().1 < cutoff_timestamp)
            .map(|e| e.key().clone())
            .collect();
        let count = keys_to_remove.len() as u32;
        for key in keys_to_remove {
            self.sent_messages.remove(&key);
        }
        Ok(count)
    }
}

// ============================================================================
// DeviceStore
// ============================================================================

#[async_trait]
impl DeviceStore for MemoryStore {
    async fn save(&self, device: &wacore::store::Device) -> Result<()> {
        let mut data = self.device_data.write().await;
        *data = Some(device.clone());
        Ok(())
    }

    async fn load(&self) -> Result<Option<wacore::store::Device>> {
        let data = self.device_data.read().await;
        Ok(data.clone())
    }

    async fn exists(&self) -> Result<bool> {
        let data = self.device_data.read().await;
        Ok(data.is_some())
    }

    async fn create(&self) -> Result<i32> {
        let id = self
            .device_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identity_roundtrip() {
        let store = MemoryStore::new();
        let key = [42u8; 32];
        store
            .put_identity("test@s.whatsapp.net", key)
            .await
            .unwrap();
        let loaded = store.load_identity("test@s.whatsapp.net").await.unwrap();
        assert_eq!(loaded, Some(key));
    }

    #[tokio::test]
    async fn session_roundtrip() {
        let store = MemoryStore::new();
        let data = b"session-data";
        store.put_session("addr", data).await.unwrap();
        let loaded = store.get_session("addr").await.unwrap();
        assert_eq!(loaded, Some(Bytes::from_static(data)));
        assert!(store.has_session("addr").await.unwrap());
        assert!(!store.has_session("missing").await.unwrap());
    }

    #[tokio::test]
    async fn device_store_roundtrip() {
        let store = MemoryStore::new();
        assert!(!store.exists().await.unwrap());
        let id = store.create().await.unwrap();
        assert_eq!(id, 0);
    }

    #[tokio::test]
    async fn prekey_operations() {
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
        let state = store.get_version("contacts").await.unwrap();
        assert_eq!(state.version, 0); // default

        let new_state = HashState {
            version: 5,
            ..Default::default()
        };
        store.set_version("contacts", new_state).await.unwrap();
        let loaded = store.get_version("contacts").await.unwrap();
        assert_eq!(loaded.version, 5);
    }

    #[tokio::test]
    async fn sender_key_devices() {
        let store = MemoryStore::new();
        assert!(
            store
                .get_sender_key_devices("group1")
                .await
                .unwrap()
                .is_empty()
        );

        store
            .set_sender_key_status("group1", &[
                ("dev1@s.whatsapp.net", true),
                ("dev2@s.whatsapp.net", false),
            ])
            .await
            .unwrap();
        let mut devices = store.get_sender_key_devices("group1").await.unwrap();
        devices.sort();
        assert_eq!(devices, vec![
            ("dev1@s.whatsapp.net".to_string(), true),
            ("dev2@s.whatsapp.net".to_string(), false),
        ]);

        store
            .delete_sender_key_device_rows(&["dev2@s.whatsapp.net"])
            .await
            .unwrap();
        assert_eq!(
            store.get_sender_key_devices("group1").await.unwrap().len(),
            1
        );

        store.clear_sender_key_devices("group1").await.unwrap();
        assert!(
            store
                .get_sender_key_devices("group1")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn lid_mapping() {
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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

        store.delete_devices("user1").await.unwrap();
        assert!(store.get_devices("user1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn max_prekey_id() {
        let store = MemoryStore::new();
        assert_eq!(store.get_max_prekey_id().await.unwrap(), 0);
        store.store_prekey(5, b"pk5", false).await.unwrap();
        store.store_prekey(10, b"pk10", true).await.unwrap();
        store.store_prekey(3, b"pk3", false).await.unwrap();
        assert_eq!(store.get_max_prekey_id().await.unwrap(), 10);
    }

    #[tokio::test]
    async fn latest_sync_key_id() {
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();
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
