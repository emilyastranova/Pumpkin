#[allow(clippy::wildcard_imports)]
use super::*;

impl PendingConnection {
    /// Registries and tags are sent in the 26.3 format; for older clients the multiversion
    /// plugin rewrites them in `JavaConnectionPacketSentEvent`.
    pub async fn handle_known_packs(&mut self, server: &Server) {
        let version = CURRENT_MC_VERSION;
        let synced = server.datapack_manager.get_all_synced_registries();
        for reg in &synced {
            self.send_packet_now(&CRegistryData::new(&reg.registry_id, &reg.registry_entries))
                .await;
        }
        let mut tags = Vec::new();
        for &key in pumpkin_data::tag::RegistryKey::NETWORK_KEYS {
            if pumpkin_data::tag::get_registry_key_tags(version, key)
                .is_some_and(|map| !map.is_empty())
            {
                tags.push(key);
            }
        }
        let block_tags = server.datapack_manager.block_tags_for_clients();
        self.send_packet_now(&CUpdateTags::new(&tags).with_block_tags(&block_tags))
            .await;
        self.send_packet_now(&CFinishConfig).await;
    }
}
