use matrix_sdk::Room;
use moltis_common::types::ReplyPayload;

use crate::error::Result;

pub async fn send_media(room: &Room, payload: &ReplyPayload) -> Result<()> {
    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

    if !payload.text.is_empty() {
        room.send(RoomMessageEventContent::text_plain(&payload.text)).await?;
    }

    if let Some(media) = &payload.media {
        let data = fetch_url_bytes(&media.url).await?;
        let content_type: mime_guess::mime::Mime = media
            .mime_type
            .parse()
            .unwrap_or(mime_guess::mime::APPLICATION_OCTET_STREAM);
        let filename = media.filename.as_deref().unwrap_or("attachment");
        let config = matrix_sdk::attachment::AttachmentConfig::new();
        room.send_attachment(filename, &content_type, data, config).await?;
    }

    Ok(())
}

async fn fetch_url_bytes(url: &str) -> Result<Vec<u8>> {
    if url.starts_with("data:") {
        let comma = url
            .find(',')
            .ok_or_else(|| crate::Error::message("invalid data URL: missing comma"))?;
        let encoded = &url[comma + 1..];
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| crate::Error::message(format!("base64 decode error: {e}")))
    } else {
        let bytes = reqwest::get(url)
            .await
            .map_err(|e| crate::Error::message(format!("failed to fetch media URL: {e}")))?
            .bytes()
            .await
            .map_err(|e| crate::Error::message(format!("failed to read media response: {e}")))?;
        Ok(bytes.to_vec())
    }
}
