//! Persistent storage backend using sled (embedded key-value database).
//!
//! Replaces `MemoryStore` so that Signal Protocol session state survives
//! restarts — users don't need to re-scan the QR code every time.
//!
//! Each account gets its own sled database at `<data_dir>/whatsapp/<account_id>/`.

use std::{fmt::Write, path::Path, sync::atomic::AtomicI32};

use {
    async_trait::async_trait,
    wacore::{
        appstate::{hash::HashState, processor::AppStateMutationMAC},
        store::{
            error::{Result, StoreError, db_err},
            traits::*,
        },
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
    skdm_recipients: sled::Tree,
    lid_mappings: sled::Tree,
    pn_mappings: sled::Tree,
    device_list_records: sled::Tree,
    sender_key_forget_marks: sled::Tree,
    base_keys: sled::Tree,
}

fn json_err(e: serde_json::Error) -> StoreError {
    StoreError::Serialization(e.to_string())
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
            skdm_recipients: db.open_tree("skdm_recipients")?,
            lid_mappings: db.open_tree("lid_mappings")?,
            pn_mappings: db.open_tree("pn_mappings")?,
            device_list_records: db.open_tree("device_list_records")?,
            sender_key_forget_marks: db.open_tree("sender_key_forget_marks")?,
            base_keys: db.open_tree("base_keys")?,
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

    async fn load_identity(&self, address: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .identities
            .get(address.as_bytes())
            .map_err(db_err)?
            .map(|v| v.to_vec()))
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        self.identities.remove(address.as_bytes()).map_err(db_err)?;
        Ok(())
    }

    async fn get_session(&self, address: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .sessions
            .get(address.as_bytes())
            .map_err(db_err)?
            .map(|v| v.to_vec()))
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

    async fn load_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        match self.prekeys.get(id.to_le_bytes()).map_err(db_err)? {
            Some(v) => {
                let (record, _uploaded): (Vec<u8>, bool) =
                    serde_json::from_slice(&v).map_err(json_err)?;
                Ok(Some(record))
            },
            None => Ok(None),
        }
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
            Some(v) => Ok(Some(serde_json::from_slice(&v).map_err(json_err)?)),
            None => Ok(None),
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let val = serde_json::to_vec(&key).map_err(json_err)?;
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
            Some(v) => Ok(serde_json::from_slice(&v).map_err(json_err)?),
            None => Ok(HashState::default()),
        }
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let val = serde_json::to_vec(&state).map_err(json_err)?;
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
        let idx_val = serde_json::to_vec(&indexes).map_err(json_err)?;
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
}

// ============================================================================
// ProtocolStore
// ============================================================================

#[async_trait]
impl ProtocolStore for SledStore {
    async fn get_skdm_recipients(&self, group_jid: &str) -> Result<Vec<String>> {
        match self
            .skdm_recipients
            .get(group_jid.as_bytes())
            .map_err(db_err)?
        {
            Some(v) => Ok(serde_json::from_slice(&v).map_err(json_err)?),
            None => Ok(Vec::new()),
        }
    }

    async fn add_skdm_recipients(&self, group_jid: &str, device_jids: &[String]) -> Result<()> {
        let mut current = self.get_skdm_recipients(group_jid).await?;
        current.extend(device_jids.iter().cloned());
        let val = serde_json::to_vec(&current).map_err(json_err)?;
        self.skdm_recipients
            .insert(group_jid.as_bytes(), val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn clear_skdm_recipients(&self, group_jid: &str) -> Result<()> {
        self.skdm_recipients
            .remove(group_jid.as_bytes())
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        match self.lid_mappings.get(lid.as_bytes()).map_err(db_err)? {
            Some(v) => Ok(Some(serde_json::from_slice(&v).map_err(json_err)?)),
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
        let val = serde_json::to_vec(entry).map_err(json_err)?;
        self.lid_mappings
            .insert(entry.lid.as_bytes(), val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        let mut result = Vec::new();
        for entry in self.lid_mappings.iter() {
            let (_, v) = entry.map_err(db_err)?;
            let mapping: LidPnMappingEntry = serde_json::from_slice(&v).map_err(json_err)?;
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
            Some(v) => Ok(Some(serde_json::from_slice(&v).map_err(json_err)?)),
            None => Ok(None),
        }
    }

    async fn mark_forget_sender_key(&self, group_jid: &str, participant: &str) -> Result<()> {
        let key = format!("{group_jid}:{participant}");
        self.sender_key_forget_marks
            .insert(key.as_bytes(), &[1u8])
            .map_err(db_err)?;
        Ok(())
    }

    async fn consume_forget_marks(&self, group_jid: &str) -> Result<Vec<String>> {
        let prefix = format!("{group_jid}:");
        let mut participants = Vec::new();
        let mut keys_to_remove = Vec::new();

        for entry in self.sender_key_forget_marks.iter() {
            let (k, _) = entry.map_err(db_err)?;
            let key_str = String::from_utf8_lossy(&k);
            if let Some(participant) = key_str.strip_prefix(&prefix) {
                participants.push(participant.to_string());
                keys_to_remove.push(k);
            }
        }
        for key in keys_to_remove {
            self.sender_key_forget_marks.remove(key).map_err(db_err)?;
        }
        Ok(participants)
    }
}

// ============================================================================
// DeviceStore
// ============================================================================

#[async_trait]
impl DeviceStore for SledStore {
    async fn save(&self, device: &wacore::store::Device) -> Result<()> {
        let val = serde_json::to_vec(device).map_err(json_err)?;
        self.device_data
            .insert(b"device", val.as_slice())
            .map_err(db_err)?;
        Ok(())
    }

    async fn load(&self) -> Result<Option<wacore::store::Device>> {
        match self.device_data.get(b"device").map_err(db_err)? {
            Some(v) => Ok(Some(serde_json::from_slice(&v).map_err(json_err)?)),
            None => Ok(None),
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

    #[tokio::test]
    async fn identity_roundtrip() {
        let store = temp_store();
        let key = [42u8; 32];
        store
            .put_identity("test@s.whatsapp.net", key)
            .await
            .unwrap();
        let loaded = store.load_identity("test@s.whatsapp.net").await.unwrap();
        assert_eq!(loaded, Some(key.to_vec()));

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
        assert_eq!(loaded, Some(data.to_vec()));
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
        assert_eq!(store.load_prekey(1).await.unwrap(), Some(b"pk1".to_vec()));
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
    async fn skdm_recipients() {
        let store = temp_store();
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
        };
        store.update_device_list(record).await.unwrap();
        let loaded = store.get_devices("user1").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().devices.len(), 1);
    }

    #[tokio::test]
    async fn forget_marks() {
        let store = temp_store();
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
        let marks = store.consume_forget_marks("group1").await.unwrap();
        assert!(marks.is_empty());
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
        }

        // Reopen and verify.
        {
            let store = SledStore::open(dir.path()).unwrap();
            let identity = store.load_identity("test@s.whatsapp.net").await.unwrap();
            assert_eq!(identity, Some(vec![1u8; 32]));
            let session = store.get_session("addr").await.unwrap();
            assert_eq!(session, Some(b"session-data".to_vec()));
            let id = store.create().await.unwrap();
            assert_eq!(id, 1); // counter persisted
        }
    }
}
