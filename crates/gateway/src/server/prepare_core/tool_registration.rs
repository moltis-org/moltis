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
    if let Some(ref tp) = state.services.telephony_plugin {
        let voice_tool =
            moltis_telephony::VoiceCallTool::new(webhook_base, std::sync::Arc::clone(tp));
        tool_registry.register(Box::new(voice_tool));
    }
}
