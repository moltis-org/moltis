// ── Shared type definitions for the moltis web UI ────────────

// Re-export all types from domain modules.
export type {
	ChannelBinding,
	ChannelCapabilities,
	ChannelDescriptor,
	ChannelReplyTarget,
	InboundMode,
} from "./channel";
// ChannelType is both a type and a runtime const object, so use plain re-export.
export { ChannelType } from "./channel";

export type {
	ActiveHoursConfig,
	CronJob,
	CronJobState,
	CronPayload,
	CronRunRecord,
	CronSandboxConfig,
	CronSchedule,
	CronStatus,
	CronWakeMode,
	GonData,
	GonKey,
	HeartbeatConfig,
	MemSnapshot,
	NavCounts,
	ResolvedIdentity,
	RunStatus,
	SandboxGonInfo,
	SessionTarget,
	SpaRoutes,
	UpdateAvailability,
} from "./gon";

export type { ModelInfo, ReasoningSuffix } from "./model";

export type { RpcError, RpcFrame, RpcResponse } from "./rpc";

export type { RpcMethod, RpcMethodMap } from "./rpc-methods";

export type { SessionMeta } from "./session";
export type {
	AbortedPartialState,
	ApprovalPayload,
	AuthCredentialsPayload,
	ChannelInfo as WsChannelInfo,
	ChatError,
	ChatPayload,
	CompactPayload,
	LocalLlmDownloadPayload,
	LocationRequestPayload,
	LogEntryPayload,
	MapLinks,
	MapPoint,
	ModelsUpdatedPayload,
	PartialMessage,
	SandboxPhasePayload,
	StreamMeta,
	ToolCallPayload,
	ToolError,
	ToolResult,
	WsErrorPayload,
	WsEventPayloadMap,
	WsFrame,
} from "./ws-events";
export { WsEventName } from "./ws-events";

// ── Additional UI types not backed by a specific Rust struct ──

/** Session token usage counters. */
export interface SessionTokens {
	input: number;
	output: number;
}

/** Project as returned by the projects.list RPC. */
export interface ProjectInfo {
	id: string;
	name?: string;
	description?: string;
	[key: string]: unknown;
}

/** Configured model entry for the provider page. */
export interface ConfiguredModel {
	id: string;
	provider: string;
	displayName?: string;
	[key: string]: unknown;
}

/** Provider metadata entry. */
export interface ProviderMeta {
	name: string;
	configured?: boolean;
	[key: string]: unknown;
}

/** Detection summary from provider auto-detect. */
export interface DetectSummary {
	[key: string]: unknown;
}

/** Detection progress update. */
export interface DetectProgress {
	[key: string]: unknown;
}

/** Channel data as returned by the channels.status RPC. */
export interface ChannelInfo {
	type: string;
	account_id?: string;
	config?: Record<string, unknown>;
	enabled?: boolean;
	[key: string]: unknown;
}

/** Sender data as returned by the server. */
export interface SenderInfo {
	id?: string;
	name?: string;
	[key: string]: unknown;
}

/** Remote node info as returned by node.list RPC. */
export interface NodeInfo {
	nodeId: string;
	displayName?: string;
	platform?: string;
	[key: string]: unknown;
}

/** MCP server info as returned by the server. */
export interface McpServerInfo {
	name?: string;
	state?: string;
	[key: string]: unknown;
}

/** A history message stored in the session history cache. */
export interface HistoryMessage {
	role?: string;
	content?: string;
	historyIndex?: number;
	messageIndex?: number;
	tool_call_id?: string;
	run_id?: string;
	[key: string]: unknown;
}
