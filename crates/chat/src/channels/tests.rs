use {
    super::*,
    async_trait::async_trait,
    moltis_channels::{ChannelReplyTarget, ChannelType},
    moltis_common::types::ReplyPayload,
    std::sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
struct SentMedia {
    account_id: String,
    to: String,
    payload: ReplyPayload,
    reply_to: Option<String>,
}

#[derive(Debug, Clone)]
struct SentText {
    account_id: String,
    to: String,
    text: String,
    reply_to: Option<String>,
}

#[derive(Default)]
struct RecordingOutbound {
    sent_media: Mutex<Vec<SentMedia>>,
    sent_text: Mutex<Vec<SentText>>,
    fail_media: bool,
    fail_text: bool,
}

#[async_trait]
impl moltis_channels::ChannelOutbound for RecordingOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        if self.fail_text {
            return Err(moltis_channels::Error::unavailable("test failure"));
        }
        self.sent_text
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(SentText {
                account_id: account_id.to_string(),
                to: to.to_string(),
                text: text.to_string(),
                reply_to: reply_to.map(ToString::to_string),
            });
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        if self.fail_media {
            return Err(moltis_channels::Error::unavailable("test failure"));
        }
        self.sent_media
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(SentMedia {
                account_id: account_id.to_string(),
                to: to.to_string(),
                payload: payload.clone(),
                reply_to: reply_to.map(ToString::to_string),
            });
        Ok(())
    }
}

fn telegram_target() -> ChannelReplyTarget {
    ChannelReplyTarget {
        ack_message_id: None,
        channel_type: ChannelType::Telegram,
        account_id: "telegram-main".into(),
        chat_id: "-100123".into(),
        message_id: Some("42".into()),
        thread_id: Some("7".into()),
    }
}

#[tokio::test]
async fn unavailable_tts_uses_successful_text_as_final_delivery() {
    let outbound = RecordingOutbound::default();
    let target = telegram_target();

    assert!(
        deliver_text_fallback(
            &outbound,
            &target,
            "-100123:7",
            "fallback transcript",
            "",
            Some("42"),
            false,
        )
        .await
    );

    let sent = outbound.sent_text.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].account_id, "telegram-main");
    assert_eq!(sent[0].to, "-100123:7");
    assert_eq!(sent[0].text, "fallback transcript");
    assert_eq!(sent[0].reply_to.as_deref(), Some("42"));
}

#[tokio::test]
async fn failed_text_fallback_is_a_final_delivery_failure() {
    let outbound = RecordingOutbound {
        fail_text: true,
        ..Default::default()
    };
    let target = telegram_target();

    assert!(!deliver_text_fallback(&outbound, &target, "-100123:7", "text", "", None, false).await);
}

#[tokio::test]
async fn generated_image_payload_dispatches_to_telegram_as_media() {
    let outbound = Arc::new(RecordingOutbound::default());
    let targets = vec![telegram_target()];

    assert!(
        dispatch_screenshot_to_targets(
            outbound.clone(),
            targets,
            "data:image/png;base64,cG5n",
            Some("Generated image: fox"),
        )
        .await
    );

    {
        let sent = outbound
            .sent_media
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].account_id, "telegram-main");
        assert_eq!(sent[0].to, "-100123:7");
        assert_eq!(sent[0].reply_to.as_deref(), Some("42"));
        assert_eq!(sent[0].payload.text, "Generated image: fox");
        let Some(media) = sent[0].payload.media.as_ref() else {
            panic!("media payload");
        };
        assert_eq!(media.mime_type, "image/png");
        assert_eq!(media.url, "data:image/png;base64,cG5n");
    }

    let failing = Arc::new(RecordingOutbound {
        fail_media: true,
        ..Default::default()
    });
    assert!(
        !dispatch_screenshot_to_targets(
            failing,
            vec![ChannelReplyTarget {
                message_id: None,
                thread_id: None,
                ..telegram_target()
            }],
            "data:image/png;base64,cG5n",
            None,
        )
        .await
    );
}

#[tokio::test]
async fn generated_image_payload_dispatches_to_matrix_as_media() {
    let outbound = Arc::new(RecordingOutbound::default());
    let targets = vec![ChannelReplyTarget {
        ack_message_id: None,
        channel_type: ChannelType::Matrix,
        account_id: "matrix-main".into(),
        chat_id: "!room:example.org".into(),
        message_id: Some("$event".into()),
        thread_id: None,
    }];

    dispatch_screenshot_to_targets(
        outbound.clone(),
        targets,
        "data:image/webp;base64,d2VicA==",
        Some("Generated image: logo"),
    )
    .await;

    let sent = outbound
        .sent_media
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].account_id, "matrix-main");
    assert_eq!(sent[0].to, "!room:example.org");
    assert_eq!(sent[0].reply_to.as_deref(), Some("$event"));
    assert_eq!(sent[0].payload.text, "Generated image: logo");
    let Some(media) = sent[0].payload.media.as_ref() else {
        panic!("media payload");
    };
    assert_eq!(media.mime_type, "image/webp");
    assert_eq!(media.url, "data:image/webp;base64,d2VicA==");
}
