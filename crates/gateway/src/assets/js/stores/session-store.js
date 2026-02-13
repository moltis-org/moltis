// ── Session store (signal-based) ─────────────────────────────
//
// Single source of truth for session data. Each session becomes a
// Session class instance with per-session signals for client-side state.

import { computed, signal } from "@preact/signals";
import { sendRpc } from "../helpers.js";

// ── Session class ────────────────────────────────────────────

export class Session {
	constructor(serverData) {
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
		this.mcpDisabled = serverData.mcpDisabled;
		this.archived = serverData.archived;
		this.activeChannel = serverData.activeChannel;
		this.version = serverData.version || 0;

		// Client signals (reactive, per-session)
		this.replying = signal(false);
		this.localUnread = signal(false);
		this.streamText = signal("");
		this.voicePending = signal(false);
		this.lastHistoryIndex = signal(-1);
		this.sessionTokens = signal({ input: 0, output: 0 });
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
	updateBadge() {
		this.badgeCount.value = this.messageCount;
	}

	/** Merge server fields, preserving client signals. Returns false if stale. */
	update(serverData) {
		var incoming = serverData.version || 0;
		if (incoming > 0 && this.version > 0 && incoming < this.version) return false;
		this.version = incoming || this.version;
		this.label = serverData.label || "";
		this.model = serverData.model || "";
		this.provider = serverData.provider || "";
		this.projectId = serverData.projectId || "";
		// Only accept server counts when they've caught up with optimistic
		// client bumps. Authoritative resets (/clear, switchSession) use
		// syncCounts() which sets messageCount directly before any fetch.
		var serverCount = serverData.messageCount || 0;
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
		this.mcpDisabled = serverData.mcpDisabled;
		this.archived = serverData.archived;
		this.activeChannel = serverData.activeChannel;
		this.updateBadge();
		this.dataVersion.value++;
		return true;
	}

	/** Optimistic bump: increment total and mark seen if active. */
	bumpCount(increment) {
		this.messageCount = (this.messageCount || 0) + increment;
		if (this.key === activeSessionKey.value) {
			this.lastSeenMessageCount = this.messageCount;
		}
		this.updateBadge();
	}

	/** Authoritative set (switchSession history, /clear). */
	syncCounts(messageCount, lastSeenMessageCount) {
		this.messageCount = messageCount;
		this.lastSeenMessageCount = lastSeenMessageCount;
		this.updateBadge();
	}

	/** Clear streaming state for this session. */
	resetStreamState() {
		this.streamText.value = "";
		this.voicePending.value = false;
		this.lastToolOutput.value = "";
	}
}

// ── Store signals ────────────────────────────────────────────
export var sessions = signal([]);
export var activeSessionKey = signal(localStorage.getItem("moltis-session") || "main");
export var switchInProgress = signal(false);

export var activeSession = computed(() => {
	var key = activeSessionKey.value;
	return sessions.value.find((s) => s.key === key) || null;
});

// ── Methods ──────────────────────────────────────────────────

/**
 * Replace the full sessions list from server data.
 * Reuses existing Session instances (matched by key) so their
 * client-side signals (replying, localUnread, streamText) are preserved.
 * New keys get fresh instances. Missing keys are dropped.
 */
export function setAll(serverSessions) {
	var existing = {};
	for (var s of sessions.value) {
		existing[s.key] = s;
	}

	var result = [];
	for (var data of serverSessions) {
		var prev = existing[data.key];
		if (prev) {
			prev.update(data);
			// Preserve client-side flags from old patched objects
			if (data._localUnread) prev.localUnread.value = true;
			if (data._replying) prev.replying.value = true;
			result.push(prev);
		} else {
			var session = new Session(data);
			if (data._localUnread) session.localUnread.value = true;
			if (data._replying) session.replying.value = true;
			result.push(session);
		}
	}

	sessions.value = result;
}

/**
 * Upsert a single session from server data.
 * Reuses existing instance when present; creates and appends when missing.
 */
export function upsert(serverData) {
	if (!(serverData && serverData.key)) return null;
	var prev = getByKey(serverData.key);
	if (prev) {
		prev.update(serverData);
		return prev;
	}
	var next = new Session(serverData);
	sessions.value = [...sessions.value, next];
	return next;
}

/** Fetch sessions from the server via RPC. */
export function fetch() {
	return sendRpc("sessions.list", {}).then((res) => {
		if (!res?.ok) return;
		setAll(res.payload || []);
	});
}

/** Notify Preact that session data changed (triggers re-render). */
export function notify() {
	sessions.value = [...sessions.value];
}

/** Look up a session by key. */
export function getByKey(key) {
	return sessions.value.find((s) => s.key === key) || null;
}

/** Set the active session key. Persists to localStorage. */
export function setActive(key) {
	activeSessionKey.value = key;
	localStorage.setItem("moltis-session", key);
}

export var sessionStore = {
	sessions,
	activeSessionKey,
	activeSession,
	switchInProgress,
	Session,
	setAll,
	upsert,
	fetch,
	getByKey,
	setActive,
	notify,
};
