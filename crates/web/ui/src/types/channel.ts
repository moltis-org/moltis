// ── Channel types ───────────────────────────────────────────────
//
// Mirrors the Rust types in `crates/channels/src/plugin.rs`.

/**
 * Channel type identifier.
 * Serialised as lowercase via `#[serde(rename_all = "lowercase")]`.
 * MsTeams is serialised as `"msteams"`.
 */
export type ChannelType = "telegram" | "whatsapp" | "msteams" | "discord" | "slack" | "matrix" | "nostr";

/**
 * Runtime constants for `ChannelType` values.
 * Use `ChannelType.Telegram` etc. instead of bare string literals.
 */
export const ChannelType = {
	Telegram: "telegram" as const,
	WhatsApp: "whatsapp" as const,
	MsTeams: "msteams" as const,
	Discord: "discord" as const,
	Slack: "slack" as const,
	Matrix: "matrix" as const,
	Nostr: "nostr" as const,
} satisfies Record<string, ChannelType>;

/**
 * How a channel receives inbound messages.
 * Serialised as snake_case via `#[serde(rename_all = "snake_case")]`.
 */
export type InboundMode = "none" | "polling" | "gateway_loop" | "socket_mode" | "webhook";

/** Static capability flags for a channel type. */
export interface ChannelCapabilities {
	inbound_mode: InboundMode;
	supports_outbound: boolean;
	supports_streaming: boolean;
	supports_interactive: boolean;
	supports_threads: boolean;
	supports_voice_ingest: boolean;
	supports_pairing: boolean;
	supports_otp: boolean;
	supports_reactions: boolean;
	supports_location: boolean;
}

/**
 * Full descriptor for a channel type, including capabilities.
 * Mirrors `ChannelDescriptor` in `crates/channels/src/plugin.rs`.
 * Injected in `gon.channel_descriptors`.
 */
export interface ChannelDescriptor {
	channel_type: ChannelType;
	display_name: string;
	capabilities: ChannelCapabilities;
}

/**
 * Where to send the LLM response back.
 * Mirrors `ChannelReplyTarget` in `crates/channels/src/plugin.rs`.
 * Stored as JSON in `SessionMeta.channelBinding`.
 */
export interface ChannelReplyTarget {
	channel_type: ChannelType;
	account_id: string;
	chat_id: string;
	message_id?: string;
	thread_id?: string;
}

/**
 * Client-side channel binding attached to a session.
 * A looser shape than `ChannelReplyTarget`, used by the session store
 * when the exact target fields are not needed.
 */
export interface ChannelBinding {
	type: string;
	account_id?: string;
	[key: string]: unknown;
}
