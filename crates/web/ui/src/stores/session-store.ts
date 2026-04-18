// ── Session store (signal-based) ─────────────────────────────
//
// Single source of truth for session data. Each session becomes a
// Session class instance with per-session signals for client-side state.

import type { Signal } from "@preact/signals";
import { computed, signal } from "@preact/signals";
import type { ChannelBinding, SessionMeta, SessionTokens } from "../types";

// ── Session class ────────────────────────────────────────────

export class Session {
	// Server fields (plain properties, set on construction/update)
	key: string;
	label: string;
	model: string;
	provider: string;
	projectId: string;
	messageCount: number;
	lastSeenMessageCount: number;
	preview: string;
	updatedAt: number;
	createdAt: number;
	worktree_branch: string;
	sandbox_enabled: boolean | undefined;
	sandbox_image: string | null;
	channelBinding: ChannelBinding | null;
	parentSessionKey: string;
	forkPoint: number | null;
	agent_id: string;
	node_id: string | null;
	mcpDisabled: boolean | undefined;
	archived: boolean | undefined;
	activeChannel: string | undefined;
	version: number;

	// Client signals (reactive, per-session)
	replying: Signal<boolean>;
	localUnread: Signal<boolean>;
	streamText: Signal<string>;
	voicePending: Signal<boolean>;
	activeRunId: Signal<string | null>;
	lastHistoryIndex: Signal<number>;
	sessionTokens: Signal<SessionTokens>;
	contextWindow: Signal<number>;
	toolsEnabled: Signal<boolean>;
	lastToolOutput: Signal<string>;
	badgeCount: Signal<number>;
	dataVersion: Signal<number>;

	constructor(serverData: SessionMeta) {
		// Server fields (plain properties, set on construction/update)
		this.key = serverData.key;
		this.label = serverData.label || "";
		this.model = serverData.model || "";
		this.provider = serverData.provider || "";
		this.projectId = serverData.projectId || "";
		this.messageCount = serverData.messageCount || 0;
		this.lastSeenMessageCount = serverData.lastSeenMessageCount || 0;
		this.preview = serverData.preview || "";
		this.updatedAt = serverData.updatedAt || 0;
		this.createdAt = serverData.createdAt || 0;
		this.worktree_branch = serverData.worktree_branch || "";
		this.sandbox_enabled = serverData.sandbox_enabled;
		this.sandbox_image = serverData.sandbox_image || null;
		this.channelBinding = serverData.channelBinding || null;
		this.parentSessionKey = serverData.parentSessionKey || "";
		this.forkPoint = serverData.forkPoint != null ? serverData.forkPoint : null;
		this.agent_id = serverData.agent_id || "main";
		this.node_id = serverData.node_id || null;
		this.mcpDisabled = serverData.mcpDisabled;
		this.archived = serverData.archived;
		this.activeChannel = serverData.activeChannel;
		this.version = serverData.version || 0;

		// Client signals (reactive, per-session)
		this.replying = signal(false);
		this.localUnread = signal(false);
		this.streamText = signal("");
		this.voicePending = signal(false);
		this.activeRunId = signal<string | null>(null);
		this.lastHistoryIndex = signal(-1);
		this.sessionTokens = signal<SessionTokens>({ input: 0, output: 0 });
		this.contextWindow = signal(0);
		this.toolsEnabled = signal(true);
		this.lastToolOutput = signal("");
		// Total message count — reactive signal that drives the sidebar badge.
		// Components read this to show/hide badge and compute unread tinting.
		this.badgeCount = signal(this.messageCount);
		// Bumped whenever plain properties change so subscribed components re-render.
		this.dataVersion = signal(0);
	}

	/** Recalculate badge from current messageCount. */
	updateBadge(): void {
		this.badgeCount.value = this.messageCount;
	}

	/** Merge server fields, preserving client signals. Returns false if stale. */
	update(serverData: SessionMeta): boolean {
		const incoming = serverData.version || 0;
		if (incoming > 0 && this.version > 0 && incoming < this.version) return false;
		this.version = incoming || this.version;
		this.label = serverData.label || "";
		this.model = serverData.model || "";
		this.provider = serverData.provider || "";
		this.projectId = serverData.projectId || "";
		// Only accept server counts when they've caught up with optimistic
		// client bumps. Authoritative resets (/clear, switchSession) use
		// syncCounts() which sets messageCount directly before any fetch.
		const serverCount = serverData.messageCount || 0;
		if (serverCount >= this.messageCount) {
			this.messageCount = serverCount;
			this.lastSeenMessageCount = serverData.lastSeenMessageCount || 0;
			this.preview = serverData.preview || "";
			this.updatedAt = serverData.updatedAt || 0;
		}
		this.createdAt = serverData.createdAt || 0;
		this.worktree_branch = serverData.worktree_branch || "";
		this.sandbox_enabled = serverData.sandbox_enabled;
		this.sandbox_image = serverData.sandbox_image || null;
		this.channelBinding = serverData.channelBinding || null;
		this.parentSessionKey = serverData.parentSessionKey || "";
		this.forkPoint = serverData.forkPoint != null ? serverData.forkPoint : null;
		this.agent_id = serverData.agent_id || "main";
		this.node_id = serverData.node_id || null;
		this.mcpDisabled = serverData.mcpDisabled;
		this.archived = serverData.archived;
		this.activeChannel = serverData.activeChannel;
		this.updateBadge();
		this.dataVersion.value++;
		return true;
	}

	/** Optimistic bump: increment total and mark seen if active. */
	bumpCount(increment: number): void {
		this.messageCount = (this.messageCount || 0) + increment;
		if (this.key === activeSessionKey.value) {
			this.lastSeenMessageCount = this.messageCount;
		}
		this.updateBadge();
	}

	/** Authoritative set (switchSession history, /clear). */
	syncCounts(messageCount: number, lastSeenMessageCount: number): void {
		this.messageCount = messageCount;
		this.lastSeenMessageCount = lastSeenMessageCount;
		this.updateBadge();
	}

	/** Clear streaming state for this session. */
	resetStreamState(): void {
		this.streamText.value = "";
		this.voicePending.value = false;
		this.activeRunId.value = null;
		this.lastToolOutput.value = "";
	}

	/** Return a plain SessionMeta snapshot of this session's server fields. */
	toMeta(): SessionMeta {
		return {
			id: 0,
			key: this.key,
			label: this.label,
			model: this.model,
			provider: this.provider,
			createdAt: this.createdAt,
			updatedAt: this.updatedAt,
			messageCount: this.messageCount,
			lastSeenMessageCount: this.lastSeenMessageCount,
			projectId: this.projectId,
			sandbox_enabled: this.sandbox_enabled,
			sandbox_image: this.sandbox_image,
			worktree_branch: this.worktree_branch,
			channelBinding: this.channelBinding,
			activeChannel: this.activeChannel,
			parentSessionKey: this.parentSessionKey,
			forkPoint: this.forkPoint,
			mcpDisabled: this.mcpDisabled,
			preview: this.preview,
			archived: this.archived,
			agent_id: this.agent_id,
			node_id: this.node_id,
			version: this.version,
		};
	}
}

// ── Store signals ────────────────────────────────────────────
export const sessions = signal<Session[]>([]);
export const activeSessionKey = signal<string>(localStorage.getItem("moltis-session") || "main");
export const switchInProgress = signal<boolean>(false);
export const refreshInProgressKey = signal<string>("");
/** Session list tab filter: "all" | "sessions" | "cron" */
export const sessionListTab = signal<string>(localStorage.getItem("moltis-session-tab") || "sessions");
export const showArchivedSessions = signal<boolean>(localStorage.getItem("moltis-show-archived-sessions") === "1");

export const activeSession = computed<Session | null>(() => {
	const key = activeSessionKey.value;
	return sessions.value.find((s) => s.key === key) || null;
});

export function compareSessionOrder(left: Session | null, right: Session | null): number {
	const leftKey = left?.key || "";
	const rightKey = right?.key || "";
	const leftMain = leftKey === "main";
	const rightMain = rightKey === "main";
	if (leftMain !== rightMain) return leftMain ? -1 : 1;

	const updatedDiff = (Number(right?.updatedAt) || 0) - (Number(left?.updatedAt) || 0);
	if (updatedDiff !== 0) return updatedDiff;

	const createdDiff = (Number(right?.createdAt) || 0) - (Number(left?.createdAt) || 0);
	if (createdDiff !== 0) return createdDiff;

	return leftKey.localeCompare(rightKey);
}

export function insertSessionInOrder(list: Session[], session: Session): Session[] {
	if (!session?.key) return Array.isArray(list) ? list.slice() : [];
	const result = Array.isArray(list) ? list.filter((entry) => entry?.key !== session.key) : [];
	result.push(session);
	result.sort(compareSessionOrder);
	return result;
}

// ── Methods ──────────────────────────────────────────────────

/**
 * Replace the full sessions list from server data.
 * Reuses existing Session instances (matched by key) so their
 * client-side signals (replying, localUnread, streamText) are preserved.
 * New keys get fresh instances. Missing keys are dropped.
 */
export function setAll(serverSessions: SessionMeta[]): void {
	const existing: Record<string, Session> = {};
	for (const s of sessions.value) {
		existing[s.key] = s;
	}

	const result: Session[] = [];
	for (const data of serverSessions) {
		const prev = existing[data.key];
		if (prev) {
			prev.update(data);
			// Preserve client-side flags from old patched objects
			if (data._localUnread) prev.localUnread.value = true;
			if (data._replying || data.replying) prev.replying.value = true;
			result.push(prev);
		} else {
			const session = new Session(data);
			if (data._localUnread) session.localUnread.value = true;
			if (data._replying || data.replying) session.replying.value = true;
			result.push(session);
		}
	}

	sessions.value = result;
}

/**
 * Upsert a single session from server data.
 * Reuses existing instance when present; creates and appends when missing.
 */
export function upsert(serverData: SessionMeta): Session | null {
	if (!serverData?.key) return null;
	const prev = getByKey(serverData.key);
	if (prev) {
		prev.update(serverData);
		sessions.value = insertSessionInOrder(sessions.value, prev);
		return prev;
	}
	const next = new Session(serverData);
	sessions.value = insertSessionInOrder(sessions.value, next);
	return next;
}

/** Remove a session by key. Returns true when a session was removed. */
export function remove(key: string): boolean {
	if (!key) return false;
	const existing = getByKey(key);
	if (!existing) return false;
	sessions.value = sessions.value.filter((session) => session.key !== key);
	if (activeSessionKey.value === key) {
		const fallback = sessions.value.find((session) => session.key === "main")?.key || sessions.value[0]?.key || "main";
		activeSessionKey.value = fallback;
		localStorage.setItem("moltis-session", fallback);
	}
	return true;
}

/** Fetch sessions from the server via HTTP (gzip-friendly). */
export function fetch(): Promise<void> {
	return window
		.fetch("/api/sessions", {
			headers: { Accept: "application/json" },
		})
		.then((response) => (response.ok ? response.json() : null))
		.then((payload: SessionMeta[] | null) => {
			if (!Array.isArray(payload)) return;
			setAll(payload);
		})
		.catch(() => {});
}

/** Notify Preact that session data changed (triggers re-render). */
export function notify(): void {
	sessions.value = [...sessions.value];
}

/** Look up a session by key. */
export function getByKey(key: string): Session | null {
	return sessions.value.find((s) => s.key === key) || null;
}

/** Set the active session key. Persists to localStorage. */
export function setActive(key: string): void {
	activeSessionKey.value = key;
	localStorage.setItem("moltis-session", key);
}

/** Set the session list tab and persist it. */
export function setSessionListTab(tab: string): void {
	sessionListTab.value = tab;
	localStorage.setItem("moltis-session-tab", tab);
}

/** Toggle whether archived sessions are shown in the sidebar. */
export function setShowArchivedSessions(show: boolean): void {
	showArchivedSessions.value = !!show;
	localStorage.setItem("moltis-show-archived-sessions", show ? "1" : "0");
}

export const sessionStore = {
	sessions,
	activeSessionKey,
	activeSession,
	switchInProgress,
	refreshInProgressKey,
	sessionListTab,
	showArchivedSessions,
	Session,
	setAll,
	upsert,
	remove,
	fetch,
	getByKey,
	setActive,
	setSessionListTab,
	setShowArchivedSessions,
	notify,
};
