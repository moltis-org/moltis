// ── WebSocket event types (discriminated union) ──────────────

/** All WebSocket event names emitted by the moltis gateway. */
export enum WsEventName {
	Chat = "chat",
	Error = "error",
	AuthCredentialsChanged = "auth.credentials_changed",
	ExecApprovalRequested = "exec.approval.requested",
	LogsEntry = "logs.entry",
	SandboxPrepare = "sandbox.prepare",
	SandboxImageBuild = "sandbox.image.build",
	SandboxImageProvision = "sandbox.image.provision",
	SandboxHostProvision = "sandbox.host.provision",
	BrowserImagePull = "browser.image.pull",
	LocalLlmDownload = "local-llm.download",
	LocalLlmLifecycle = "local-llm.lifecycle",
	ModelsUpdated = "models.updated",
	LocationRequest = "location.request",
	NetworkAuditEntry = "network.audit.entry",
	// Additional onEvent() events
	Tick = "tick",
	Session = "session",
	Channel = "channel",
	Presence = "presence",
	UpdateAvailable = "update.available",
	McpStatus = "mcp.status",
	HooksStatus = "hooks.status",
	MetricsUpdate = "metrics.update",
	SkillsInstallProgress = "skills.install.progress",
	PushSubscriptions = "push.subscriptions",
	NodePairRequested = "node.pair.requested",
	NodePairResolved = "node.pair.resolved",
	DevicePairResolved = "device.pair.resolved",
	NodeTelemetry = "node.telemetry",
}

// ── Payload interfaces ───────────────────────────────────────

export interface ToolResult {
	stdout?: string;
	stderr?: string;
	exit_code?: number;
	screenshot?: string;
	screenshot_scale?: number;
	document_ref?: string;
	filename?: string;
	mime_type?: string;
	size_bytes?: number;
	points?: MapPoint[];
	label?: string;
	map_links?: MapLinks;
}

export interface ToolError {
	detail?: string;
	message?: string;
	retryAfterMs?: number;
	type?: string;
}

export interface MapLinks {
	url?: string;
	google_maps?: string;
	apple_maps?: string;
	openstreetmap?: string;
	[key: string]: unknown;
}

export interface MapPoint {
	label?: string;
	latitude?: number;
	longitude?: number;
	map_links?: MapLinks;
}

export interface ToolCallPayload {
	runId?: string;
	toolCallId?: string;
	toolName?: string;
	arguments?: Record<string, unknown>;
	executionMode?: string;
	messageIndex?: number;
	sessionKey?: string;
	success?: boolean;
	result?: ToolResult;
	error?: ToolError;
}

export interface ChatError {
	title?: string;
	detail?: string;
	message?: string;
	type?: string;
	retryAfterMs?: number;
	canContinue?: boolean;
}

export interface ChannelInfo {
	audio_filename?: string;
	[key: string]: unknown;
}

export interface PartialMessage {
	content?: string;
	reasoning?: string;
	model?: string;
	provider?: string;
	inputTokens?: number;
	outputTokens?: number;
	durationMs?: number;
	requestInputTokens?: number;
	requestOutputTokens?: number;
	requestCacheReadTokens?: number;
	requestCacheWriteTokens?: number;
	audio?: string;
	run_id?: string;
	created_at?: number;
}

export interface ChatPayload {
	state?: string;
	sessionKey?: string;
	runId?: string;
	text?: string;
	model?: string;
	provider?: string;
	inputTokens?: number;
	outputTokens?: number;
	cacheReadTokens?: number;
	cacheWriteTokens?: number;
	durationMs?: number;
	requestInputTokens?: number;
	requestOutputTokens?: number;
	requestCacheReadTokens?: number;
	requestCacheWriteTokens?: number;
	reasoning?: string;
	audio?: string;
	audioWarning?: string | null;
	replyMedium?: string;
	messageIndex?: number;
	toolCallId?: string;
	toolName?: string;
	arguments?: Record<string, unknown>;
	executionMode?: string;
	success?: boolean;
	result?: ToolResult;
	error?: ChatError;
	message?: string;
	channel?: ChannelInfo;
	title?: string;
	phase?: string;
	mode?: string;
	seq?: number;
	retryAfterMs?: number;
	partialMessage?: PartialMessage;
	canContinue?: boolean;
}

export interface CompactPayload {
	sessionKey?: string;
	phase?: string;
	error?: string;
	mode?: string;
	[key: string]: unknown;
}

export interface ApprovalPayload {
	requestId: string;
	command: string;
}

export interface LogEntryPayload {
	level?: string;
	[key: string]: unknown;
}

export interface SandboxPhasePayload {
	phase?: string;
	error?: string;
	tag?: string;
	built?: boolean;
	count?: number;
	installed?: number;
	skipped?: number;
	image?: string;
	package_count?: number;
}

export interface LocalLlmDownloadPayload {
	displayName?: string;
	modelId?: string;
	error?: string;
	complete?: boolean;
	progress?: number;
	downloaded?: number;
	total?: number;
}

export interface LocalLlmLifecyclePayload {
	modelId: string;
	state: "loading" | "loaded" | "unloading" | "unloaded";
	modelSizeBytes?: number;
	totalLoadedBytes?: number;
	reason?: "idle" | "manual";
}

export interface ModelsUpdatedPayload {
	phase?: string;
	[key: string]: unknown;
}

export interface WsErrorPayload {
	message?: string;
}

export interface LocationRequestPayload {
	requestId?: string;
	precision?: string;
}

export interface AuthCredentialsPayload {
	reason?: string;
}

export interface WsFrame {
	type: string;
	event?: string;
	payload?: Record<string, unknown>;
	stream?: unknown;
	done?: unknown;
	channel?: unknown;
}

export interface StreamMeta {
	stream: unknown;
	done: unknown;
	channel: unknown;
}

export interface AbortedPartialState {
	partial: PartialMessage | null;
	partialText: string;
	partialReasoning: string;
	hasVisiblePartial: boolean;
}

/** Maps event names to their payload types. */
export interface WsEventPayloadMap {
	[WsEventName.Chat]: ChatPayload;
	[WsEventName.Error]: WsErrorPayload;
	[WsEventName.AuthCredentialsChanged]: AuthCredentialsPayload;
	[WsEventName.ExecApprovalRequested]: ApprovalPayload;
	[WsEventName.LogsEntry]: LogEntryPayload;
	[WsEventName.SandboxPrepare]: SandboxPhasePayload;
	[WsEventName.SandboxImageBuild]: SandboxPhasePayload;
	[WsEventName.SandboxImageProvision]: SandboxPhasePayload;
	[WsEventName.SandboxHostProvision]: SandboxPhasePayload;
	[WsEventName.BrowserImagePull]: SandboxPhasePayload;
	[WsEventName.LocalLlmDownload]: LocalLlmDownloadPayload;
	[WsEventName.LocalLlmLifecycle]: LocalLlmLifecyclePayload;
	[WsEventName.ModelsUpdated]: ModelsUpdatedPayload;
	[WsEventName.LocationRequest]: LocationRequestPayload;
	[WsEventName.NetworkAuditEntry]: Record<string, unknown>;
	[WsEventName.Tick]: Record<string, unknown>;
	[WsEventName.Session]: Record<string, unknown>;
	[WsEventName.Channel]: Record<string, unknown>;
	[WsEventName.Presence]: Record<string, unknown>;
	[WsEventName.UpdateAvailable]: Record<string, unknown>;
	[WsEventName.McpStatus]: Record<string, unknown>;
	[WsEventName.HooksStatus]: Record<string, unknown>;
	[WsEventName.MetricsUpdate]: Record<string, unknown>;
	[WsEventName.SkillsInstallProgress]: Record<string, unknown>;
	[WsEventName.PushSubscriptions]: Record<string, unknown>;
	[WsEventName.NodePairRequested]: Record<string, unknown>;
	[WsEventName.NodePairResolved]: Record<string, unknown>;
	[WsEventName.DevicePairResolved]: Record<string, unknown>;
	[WsEventName.NodeTelemetry]: Record<string, unknown>;
}
