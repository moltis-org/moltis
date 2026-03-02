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
    app_state_versions: Arc<DashMap<String, HashState>>,
    /// Keyed by `"{name}:{version}:{hex(index_mac)}"`.
    mutation_macs: Arc<DashMap<String, Vec<u8>>>,
    /// Keyed by `"{name}:{version}"` → list of index_macs stored at that version.
    mutation_mac_indexes: Arc<DashMap<String, Vec<Vec<u8>>>>,
    device_data: Arc<tokio::sync::RwLock<Option<wacore::store::Device>>>,
    device_id: Arc<std::sync::atomic::AtomicI32>,
    skdm_recipients: Arc<DashMap<String, Vec<String>>>,
    lid_mappings: Arc<DashMap<String, LidPnMappingEntry>>,
    /// Phone number → LID reverse index.
    pn_mappings: Arc<DashMap<String, String>>,
    device_list_records: Arc<DashMap<String, DeviceListRecord>>,
    /// Keyed by `"{group_jid}:{participant}"`.
    sender_key_forget_marks: Arc<DashMap<String, bool>>,
    /// Base keys keyed by `"{address}:{message_id}"`.
    base_keys: Arc<DashMap<String, Vec<u8>>>,
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

    async fn load_identity(&self, address: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.identities.get(address).map(|v| v.value().clone()))
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        self.identities.remove(address);
        Ok(())
    }

    async fn get_session(&self, address: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.sessions.get(address).map(|v| v.value().clone()))
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

    async fn load_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        Ok(self.prekeys.get(&id).map(|v| v.value().0.clone()))
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        self.prekeys.remove(&id);
        Ok(())
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
}

// ============================================================================
// ProtocolStore
// ============================================================================

#[async_trait]
impl ProtocolStore for MemoryStore {
    async fn get_skdm_recipients(&self, group_jid: &str) -> Result<Vec<String>> {
        Ok(self
            .skdm_recipients
            .get(group_jid)
            .map(|v| v.value().clone())
            .unwrap_or_default())
    }

    async fn add_skdm_recipients(&self, group_jid: &str, device_jids: &[String]) -> Result<()> {
        self.skdm_recipients
            .entry(group_jid.to_string())
            .or_default()
            .extend(device_jids.iter().cloned());
        Ok(())
    }

    async fn clear_skdm_recipients(&self, group_jid: &str) -> Result<()> {
        self.skdm_recipients.remove(group_jid);
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

    async fn mark_forget_sender_key(&self, group_jid: &str, participant: &str) -> Result<()> {
        let key = format!("{group_jid}:{participant}");
        self.sender_key_forget_marks.insert(key, true);
        Ok(())
    }

    async fn consume_forget_marks(&self, group_jid: &str) -> Result<Vec<String>> {
        let prefix = format!("{group_jid}:");
        let keys: Vec<String> = self
            .sender_key_forget_marks
            .iter()
            .filter(|e| e.key().starts_with(&prefix))
            .map(|e| e.key().clone())
            .collect();

        let mut participants = Vec::new();
        for key in keys {
            self.sender_key_forget_marks.remove(&key);
            if let Some(participant) = key.strip_prefix(&prefix) {
                participants.push(participant.to_string());
            }
        }
        Ok(participants)
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
        assert_eq!(loaded, Some(key.to_vec()));
    }

    #[tokio::test]
    async fn session_roundtrip() {
        let store = MemoryStore::new();
        let data = b"session-data";
        store.put_session("addr", data).await.unwrap();
        let loaded = store.get_session("addr").await.unwrap();
        assert_eq!(loaded, Some(data.to_vec()));
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
        assert_eq!(store.load_prekey(1).await.unwrap(), Some(b"pk1".to_vec()));
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
    async fn skdm_recipients() {
        let store = MemoryStore::new();
        let recips = store.get_skdm_recipients("group1").await.unwrap();
        assert!(recips.is_empty());

        store
            .add_skdm_recipients("group1", &["dev1".into(), "dev2".into()])
            .await
            .unwrap();
        let recips = store.get_skdm_recipients("group1").await.unwrap();
        assert_eq!(recips.len(), 2);

        store.clear_skdm_recipients("group1").await.unwrap();
        assert!(
            store
                .get_skdm_recipients("group1")
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
        };
        store.update_device_list(record).await.unwrap();
        let loaded = store.get_devices("user1").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().devices.len(), 1);
    }

    #[tokio::test]
    async fn forget_marks() {
        let store = MemoryStore::new();
        store
            .mark_forget_sender_key("group1", "user_a")
            .await
            .unwrap();
        store
            .mark_forget_sender_key("group1", "user_b")
            .await
            .unwrap();
        let marks = store.consume_forget_marks("group1").await.unwrap();
        assert_eq!(marks.len(), 2);
        // Consumed — should be empty now.
        let marks = store.consume_forget_marks("group1").await.unwrap();
        assert!(marks.is_empty());
    }
}
