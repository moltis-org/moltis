#[cfg(feature = "telephony")]
pub(super) async fn register_voice_call_tool(
    tool_registry: &mut moltis_agents::tool_registry::ToolRegistry,
    state: &crate::state::GatewayState,
) {
    let webhook_base = state
        .config
        .server
        .effective_external_url()
        .unwrap_or_default();
    let voice_tool = moltis_telephony::VoiceCallTool::new(webhook_base);

    if let Some(ref tp) = state.services.telephony_plugin {
        use moltis_channels::ChannelPlugin as _;
        let plugin = tp.read().await;
        for aid in plugin.account_ids() {
            if let (Some(mgr), Some(from)) = (plugin.call_manager(&aid), plugin.caller_number(&aid))
            {
                let account_webhook_base = plugin.account_config_json(&aid).and_then(|config| {
                    config["webhook_url"]
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                });
                voice_tool
                    .add_manager(aid, mgr, from, account_webhook_base)
                    .await;
            }
        }
    }

    tool_registry.register(Box::new(voice_tool));
}
