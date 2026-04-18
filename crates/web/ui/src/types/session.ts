// ── Session types ──────────────────────────────────────────────
//
// Mirrors the JSON shape produced by `build_recent_sessions_snapshot()`
// in `crates/web/src/templates.rs` and `sessions.list` in
// `crates/gateway/src/session/service.rs`.

import type { ChannelBinding } from "./channel";

/**
 * Session metadata as returned by the server.
 *
 * Field casing is mixed: some fields are camelCase (from the JSON builder
 * in templates.rs) while others are snake_case (from the session metadata
 * store). Both forms are represented here to match what the JS side reads.
 */
export interface SessionMeta {
	id: number;
	key: string;
	label?: string;
	model?: string;
	provider?: string;
	createdAt?: number;
	updatedAt?: number;
	messageCount?: number;
	lastSeenMessageCount?: number;
	projectId?: string;
	sandbox_enabled?: boolean;
	sandbox_image?: string | null;
	worktree_branch?: string;
	channelBinding?: ChannelBinding | null;
	activeChannel?: string;
	parentSessionKey?: string | null;
	forkPoint?: number | null;
	mcpDisabled?: boolean;
	preview?: string | null;
	archived?: boolean;
	/** Snake_case form emitted by the server. */
	agent_id?: string;
	/** CamelCase alias emitted alongside agent_id. */
	agentId?: string;
	node_id?: string | null;
	version?: number;
	/** Client-side flag: set transiently during setAll merges. */
	_localUnread?: boolean;
	/** Client-side flag: set transiently during setAll merges. */
	_replying?: boolean;
	/** Legacy client-side flag. */
	replying?: boolean;
}
