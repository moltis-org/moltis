/// TTL-based media cleanup (default 2 minutes).
pub async fn clean_old_media(_media_dir: &std::path::Path, _ttl_secs: u64) -> crate::Result<u64> {
    todo!("delete files older than TTL, return count deleted")
}
