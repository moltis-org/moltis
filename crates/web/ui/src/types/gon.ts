// ── GonData types -- mirrors the Rust GonData struct in templates.rs ──

import type { ChannelDescriptor } from "./channel";
import type { SessionMeta } from "./session";

// ── SPA routes ──────────────────────────────────────────────

export interface SpaRoutes {
	chats: string;
	settings: string;
	providers: string;
	security: string;
	profile: string;
	config: string;
	logs: string;
	nodes: string;
	onboarding: string;
	projects: string;
	skills: string;
	crons: string;
	monitoring: string;
	graphql: string;
}

// ── Nav counts ──────────────────────────────────────────────

export interface NavCounts {
	projects: number;
	providers: number;
	channels: number;
	skills: number;
	mcp: number;
	crons: number;
	hooks: number;
}

// ── Memory snapshot ─────────────────────────────────────────

/** Serialised with camelCase via `#[serde(rename_all = "camelCase")]`. */
export interface MemSnapshot {
	process: number;
	localLlamaCpp?: number;
	available: number;
	total: number;
}

// ── Sandbox gon info ────────────────────────────────────────

export interface SandboxGonInfo {
	backend: string;
	os: string;
	default_image: string;
	image_building: boolean;
}

// ── Identity ────────────────────────────────────────────────

/**
 * Resolved identity combining agent identity and user profile.
 * Mirrors `ResolvedIdentity` in `crates/config/src/schema.rs`.
 *
 * The `user_timezone` field is added by the onboarding service's
 * `identity_get` handler and is not on the Rust struct directly.
 */
export interface ResolvedIdentity {
	name: string;
	emoji?: string;
	theme?: string;
	soul?: string;
	user_name?: string;
	user_timezone?: string;
}

// ── Heartbeat ───────────────────────────────────────────────

/** Active hours window for heartbeat scheduling. */
export interface ActiveHoursConfig {
	start: string;
	end: string;
	timezone: string;
}

/**
 * Heartbeat configuration.
 * Mirrors `HeartbeatConfig` in `crates/config/src/schema/system.rs`.
 * Serialised with default serde (snake_case field names).
 */
export interface HeartbeatConfig {
	enabled: boolean;
	every: string;
	model?: string;
	prompt?: string;
	ack_max_chars: number;
	active_hours: ActiveHoursConfig;
	deliver: boolean;
	channel?: string;
	to?: string;
	sandbox_enabled: boolean;
	sandbox_image?: string;
}

// ── Cron types ──────────────────────────────────────────────

/**
 * All cron types use `#[serde(rename_all = "camelCase")]`.
 */

/** How a cron job is scheduled. Tagged union with `kind` discriminant. */
export type CronSchedule =
	| { kind: "at"; atMs: number }
	| { kind: "every"; everyMs: number; anchorMs?: number }
	| { kind: "cron"; expr: string; tz?: string };

/** What happens when a cron job fires. Tagged union with `kind` discriminant. */
export type CronPayload =
	| { kind: "systemEvent"; text: string }
	| {
			kind: "agentTurn";
			message: string;
			model?: string;
			timeoutSecs?: number;
			deliver: boolean;
			channel?: string;
			to?: string;
	  };

/** Where the job executes. */
export type SessionTarget = "main" | "isolated" | string;

/** Outcome of a single job run. */
export type RunStatus = "ok" | "error" | "skipped";

/** Whether to wake the heartbeat after a cron job completes. */
export type CronWakeMode = "now" | "nextHeartbeat";

/** Sandbox configuration for a cron job. */
export interface CronSandboxConfig {
	enabled: boolean;
	image?: string;
	autoPruneContainer?: boolean;
}

/** Mutable runtime state of a cron job. */
export interface CronJobState {
	nextRunAtMs?: number;
	runningAtMs?: number;
	lastRunAtMs?: number;
	lastStatus?: RunStatus;
	lastError?: string;
	lastDurationMs?: number;
}

/** A scheduled cron job. */
export interface CronJob {
	id: string;
	name: string;
	enabled: boolean;
	deleteAfterRun: boolean;
	schedule: CronSchedule;
	payload: CronPayload;
	sessionTarget: SessionTarget;
	state: CronJobState;
	sandbox: CronSandboxConfig;
	wakeMode: CronWakeMode;
	system: boolean;
	createdAtMs: number;
	updatedAtMs: number;
}

/** Record of a completed cron run. */
export interface CronRunRecord {
	jobId: string;
	startedAtMs: number;
	finishedAtMs: number;
	status: RunStatus;
	error?: string;
	durationMs: number;
	output?: string;
	inputTokens?: number;
	outputTokens?: number;
	sessionKey?: string;
}

/** Summary status of the cron system. */
export interface CronStatus {
	running: boolean;
	jobCount: number;
	enabledCount: number;
	nextRunAtMs?: number;
}

// ── Update availability ─────────────────────────────────────

export interface UpdateAvailability {
	available: boolean;
	latest_version?: string;
	release_url?: string;
}

// ── GonData ─────────────────────────────────────────────────

/**
 * Server-side data injected into every page as `window.__MOLTIS__`.
 * Mirrors the Rust `GonData` struct in `crates/web/src/templates.rs`.
 */
export interface GonData {
	identity: ResolvedIdentity;
	version: string;
	port: number;
	counts: NavCounts;
	crons: CronJob[];
	cron_status: CronStatus;
	heartbeat_config: HeartbeatConfig;
	heartbeat_runs: CronRunRecord[];
	voice_enabled: boolean;
	graphql_enabled: boolean;
	terminal_enabled: boolean;
	git_branch?: string;
	mem: MemSnapshot;
	deploy_platform?: string;
	channels_offered: string[];
	channel_descriptors: ChannelDescriptor[];
	channel_storage_db_path: string;
	update: UpdateAvailability;
	sandbox: SandboxGonInfo;
	routes: SpaRoutes;
	started_at: number;
	openclaw_detected: boolean;
	sessions_recent: SessionMeta[];
	agents: unknown[];
	webhooks: unknown[];
	webhook_profiles: unknown[];
	vault_status?: string;
}

/** Key of GonData for use with get/set/onChange. */
export type GonKey = keyof GonData;
