// ── Sessions: list, switch, status helpers ──────────────────

import { sendRpc } from "./helpers";
import { currentPrefix, navigate, sessionPath } from "./router";
import { clearSessionHistoryCache } from "./sessions/session-history";
import { updateChatSessionHeader } from "./sessions/session-render";
import { switchSession } from "./sessions/session-switch";
import * as S from "./state";
import { projectStore } from "./stores/project-store";
import { clearSessionHistory } from "./stores/session-history-cache";
import { sessionStore } from "./stores/session-store";
import type { SessionMeta } from "./types";
import { confirmDialog } from "./ui";

// ── Re-exports from sub-modules ──────────────────────────────

export {
	cacheOutgoingUserMessage,
	cacheSessionHistoryMessage,
	clearHistoryPaginationState,
	clearSessionHistoryCache,
} from "./sessions/session-history";
export type { SearchContext } from "./sessions/session-render";
export {
	appendLastMessageTimestamp,
	refreshWelcomeCardIfNeeded,
	updateChatSessionHeader,
} from "./sessions/session-render";

export { clearActiveSession, switchSession } from "./sessions/session-switch";

// ── Types ────────────────────────────────────────────────────

interface SessionListPage {
	sessions: SessionMeta[];
	hasMore: boolean;
	nextCursor: number | null;
	total: number | null;
}

interface SessionListPaging {
	hasMore: boolean;
	nextCursor: number | null;
	total: number | null;
	loading: boolean;
}

// ── Module-level state ──────────────────────────────────────

const SESSION_PREVIEW_MAX_CHARS = 200;
const SESSION_LIST_PAGE_LIMIT = 40;
const SESSION_LIST_REFRESH_LIMIT_MAX = 200;
const SESSION_LIST_SCROLL_THRESHOLD = 220;
const sessionListPaging: SessionListPaging = {
	hasMore: false,
	nextCursor: null,
	total: null,
	loading: false,
};
let sessionListPendingRefresh = false;
let sessionListScrollEl: HTMLElement | null = null;
let sessionListScrollRaf = 0;

function truncateSessionPreview(text: string | null | undefined): string {
	const trimmed = (text || "").trim();
	if (!trimmed) return "";
	const chars = Array.from(trimmed);
	if (chars.length <= SESSION_PREVIEW_MAX_CHARS) return trimmed;
	return `${chars.slice(0, SESSION_PREVIEW_MAX_CHARS).join("")}\u2026`;
}

// ── Fetch & render ──────────────────────────────────────────

export function fetchSessions(): void {
	ensureSessionListScrollBinding();
	if (sessionListPaging.loading) {
		sessionListPendingRefresh = true;
		return;
	}

	sessionListPaging.loading = true;
	const loadedCount = Array.isArray(S.sessions) ? S.sessions.length : 0;
	const refreshLimit = Math.max(
		SESSION_LIST_PAGE_LIMIT,
		Math.min(
			Number.isInteger(loadedCount) && loadedCount > 0 ? loadedCount : SESSION_LIST_PAGE_LIMIT,
			SESSION_LIST_REFRESH_LIMIT_MAX,
		),
	);

	void fetchSessionListPage({ limit: refreshLimit })
		.then((page) => {
			const merged = mergeSessionListPage(S.sessions as SessionMeta[], page.sessions, false);
			applySessionList(merged);
			applySessionListPaging(page);
		})
		.catch(() => {})
		.finally(() => {
			sessionListPaging.loading = false;
			if (sessionListPendingRefresh) {
				sessionListPendingRefresh = false;
				fetchSessions();
				return;
			}
			maybeLoadMoreSessionsFromScroll();
		});
}

function toValidCursor(value: unknown): number | null {
	const parsed = Number(value);
	if (!Number.isInteger(parsed) || parsed < 0) return null;
	return parsed;
}

function parseSessionListPayload(payload: unknown): SessionListPage {
	if (Array.isArray(payload)) {
		return {
			sessions: payload as SessionMeta[],
			hasMore: false,
			nextCursor: null,
			total: payload.length,
		};
	}

	const obj = payload as Record<string, unknown> | null;
	const list = Array.isArray(obj?.sessions) ? (obj?.sessions as SessionMeta[]) : [];
	const nextCursor = toValidCursor(obj?.nextCursor);
	const hasMore = obj?.hasMore === true && nextCursor !== null;
	const total = Number(obj?.total);
	return {
		sessions: list,
		hasMore,
		nextCursor: hasMore ? nextCursor : null,
		total: Number.isInteger(total) && total >= 0 ? total : null,
	};
}

function mergeSessionListPage(
	existingSessions: SessionMeta[],
	incomingSessions: SessionMeta[],
	append: boolean,
): SessionMeta[] {
	const existing = Array.isArray(existingSessions) ? existingSessions : [];
	const incoming = Array.isArray(incomingSessions) ? incomingSessions : [];

	const oldByKey: Record<string, SessionMeta> = {};
	for (const old of existing) {
		if (!old?.key) continue;
		oldByKey[old.key] = old;
	}

	function withLocalFlags(session: SessionMeta): SessionMeta {
		if (!session?.key) return session;
		const prev = oldByKey[session.key];
		if (!prev) return session;
		const merged = { ...session };
		if (prev._localUnread) merged._localUnread = true;
		if (prev._replying) merged._replying = true;
		return merged;
	}

	if (!append) {
		return incoming.map((session) => withLocalFlags(session));
	}

	const result = existing.slice();
	const indexByKey: Record<string, number> = {};
	for (let i = 0; i < result.length; i += 1) {
		const key = result[i]?.key;
		if (!key) continue;
		indexByKey[key] = i;
	}

	for (const session of incoming) {
		if (!session?.key) continue;
		const next = withLocalFlags(session);
		const idx = indexByKey[session.key];
		if (Number.isInteger(idx)) {
			result[idx] = { ...result[idx], ...next };
			continue;
		}
		indexByKey[session.key] = result.length;
		result.push(next);
	}

	return result;
}

function applySessionList(sessions: SessionMeta[]): void {
	// Update session store (source of truth) -- version guard
	// inside Session.update() prevents stale data from overwriting.
	sessionStore.setAll(sessions);
	// Dual-write to state.js for backward compat
	S.setSessions(sessions);
	renderSessionList();
	updateChatSessionHeader();
}

function applySessionListPaging(page: SessionListPage): void {
	sessionListPaging.hasMore = page.hasMore === true && Number.isInteger(page.nextCursor);
	sessionListPaging.nextCursor = sessionListPaging.hasMore ? page.nextCursor : null;
	sessionListPaging.total = Number.isInteger(page.total) ? page.total : null;
}

async function fetchSessionListPage(options?: { cursor?: number; limit?: number }): Promise<SessionListPage> {
	const opts = options || {};
	const query = new URLSearchParams();
	if (Number.isInteger(opts.cursor) && (opts.cursor as number) >= 0) {
		query.set("cursor", String(opts.cursor));
	}
	if (Number.isInteger(opts.limit) && (opts.limit as number) > 0) {
		query.set("limit", String(opts.limit));
	}

	let url = "/api/sessions";
	const qs = query.toString();
	if (qs) url += `?${qs}`;

	const response = await fetch(url, {
		headers: { Accept: "application/json" },
	});
	let payload: unknown = null;
	try {
		payload = await response.json();
	} catch {
		payload = null;
	}
	if (!response.ok) {
		throw new Error(`Failed to fetch sessions (${response.status})`);
	}
	return parseSessionListPayload(payload);
}

function shouldLoadMoreSessions(): boolean {
	const el = S.$("sessionList");
	if (!el) return false;
	if (el.clientHeight <= 0) return false;
	if (sessionListPaging.loading) return false;
	if (!(sessionListPaging.hasMore && Number.isInteger(sessionListPaging.nextCursor))) return false;
	const distance = el.scrollHeight - (el.scrollTop + el.clientHeight);
	return distance <= SESSION_LIST_SCROLL_THRESHOLD;
}

async function loadMoreSessionsPage(): Promise<void> {
	if (!shouldLoadMoreSessions()) return;
	sessionListPaging.loading = true;
	try {
		const page = await fetchSessionListPage({
			cursor: sessionListPaging.nextCursor as number,
			limit: SESSION_LIST_PAGE_LIMIT,
		});
		const merged = mergeSessionListPage(S.sessions as SessionMeta[], page.sessions, true);
		applySessionList(merged);
		if (page.sessions.length === 0) {
			applySessionListPaging({
				hasMore: false,
				nextCursor: null,
				total: page.total,
				sessions: [],
			});
		} else {
			applySessionListPaging(page);
		}
	} catch {
		// Keep existing list on transient paging errors.
	} finally {
		sessionListPaging.loading = false;
		if (sessionListPendingRefresh) {
			sessionListPendingRefresh = false;
			fetchSessions();
		} else {
			maybeLoadMoreSessionsFromScroll();
		}
	}
}

function maybeLoadMoreSessionsFromScroll(): void {
	if (!shouldLoadMoreSessions()) return;
	void loadMoreSessionsPage();
}

function handleSessionListScroll(): void {
	if (sessionListScrollRaf) return;
	sessionListScrollRaf = requestAnimationFrame(() => {
		sessionListScrollRaf = 0;
		maybeLoadMoreSessionsFromScroll();
	});
}

function ensureSessionListScrollBinding(): void {
	const nextEl = S.$("sessionList");
	if (sessionListScrollEl === nextEl) return;
	if (sessionListScrollEl) {
		sessionListScrollEl.removeEventListener("scroll", handleSessionListScroll);
	}
	sessionListScrollEl = nextEl;
	if (!sessionListScrollEl) return;
	sessionListScrollEl.addEventListener("scroll", handleSessionListScroll, { passive: true });
}

export function markSessionLocallyCleared(key: string): void {
	if (!key) return;
	const now = Date.now();

	const session = sessionStore.getByKey(key);
	if (session) {
		session.syncCounts(0, 0);
		session.preview = "";
		session.updatedAt = now;
		session.replying.value = false;
		session.activeRunId.value = null;
		session.lastHistoryIndex.value = -1;
		const localVersion = Number.isInteger(session.version) ? session.version : 0;
		session.version = localVersion + 1;
		session.dataVersion.value++;
	}

	const legacy = (S.sessions as SessionMeta[]).find((s) => s.key === key);
	if (legacy) {
		legacy.messageCount = 0;
		legacy.lastSeenMessageCount = 0;
		legacy.preview = "";
		legacy.updatedAt = now;
		legacy._localUnread = false;
		legacy._replying = false;
		const legacyVersion = Number.isInteger(legacy.version) ? (legacy.version as number) : 0;
		legacy.version = legacyVersion + 1;
	}
}

// ── Session list ─────────────────────────────────────────────
// The Preact SessionList component is mounted once from app.js and
// auto-rerenders from signals.

export function renderSessionList(): void {
	ensureSessionListScrollBinding();
	maybeLoadMoreSessionsFromScroll();
}

// ── Status helpers ──────────────────────────────────────────

export function setSessionReplying(key: string, replying: boolean): void {
	// Update store signal -- Preact SessionList re-renders automatically.
	const session = sessionStore.getByKey(key);
	if (session) session.replying.value = replying;
	// Dual-write: update plain S.sessions object
	const entry = (S.sessions as SessionMeta[]).find((s) => s.key === key);
	if (entry) entry._replying = replying;
}

export function setSessionActiveRunId(key: string, runId: string | null): void {
	const session = sessionStore.getByKey(key);
	if (session) session.activeRunId.value = runId || null;
	const entry = (S.sessions as SessionMeta[]).find((s) => s.key === key);
	if (entry) (entry as SessionMeta & { _activeRunId?: string | null })._activeRunId = runId || null;
}

export function setSessionUnread(key: string, unread: boolean): void {
	// Update store signal -- Preact SessionList re-renders automatically.
	const session = sessionStore.getByKey(key);
	if (session) session.localUnread.value = unread;
	// Dual-write: update plain S.sessions object
	const entry = (S.sessions as SessionMeta[]).find((s) => s.key === key);
	if (entry) entry._localUnread = unread;
}

export function bumpSessionCount(key: string, increment: number): void {
	// Update store -- bumpCount bumps dataVersion for automatic re-render.
	const session = sessionStore.getByKey(key);
	if (session) {
		session.bumpCount(increment);
	}

	// Dual-write: update the underlying S.sessions data.
	const entry = (S.sessions as SessionMeta[]).find((s) => s.key === key);
	if (entry) {
		entry.messageCount = (entry.messageCount || 0) + increment;
		if (key === S.activeSessionKey) {
			entry.lastSeenMessageCount = entry.messageCount;
		}
	}
}

/** Set first-message preview optimistically so sidebar updates without reload. */
export function seedSessionPreviewFromUserText(key: string, text: string): void {
	const preview = truncateSessionPreview(text);
	if (!preview) return;
	const now = Date.now();

	const session = sessionStore.getByKey(key);
	if (session && !session.preview) {
		session.preview = preview;
		session.updatedAt = now;
		session.dataVersion.value++;
	}

	const entry = (S.sessions as SessionMeta[]).find((s) => s.key === key);
	if (entry && !entry.preview) {
		entry.preview = preview;
		entry.updatedAt = now;
	}
}

export function removeSessionFromClientState(
	key: string,
	options?: { nextKey?: string; navigateIfActive?: boolean },
): boolean {
	const opts = options || {};
	if (!key) return false;
	const removedActive = sessionStore.activeSessionKey.value === key;
	const removed = sessionStore.remove(key);
	if (!removed) return false;
	const nextKey = opts.nextKey || sessionStore.activeSessionKey.value || "main";
	if (removedActive && nextKey !== sessionStore.activeSessionKey.value) sessionStore.setActive(nextKey);
	clearSessionHistoryCache(key);
	S.setSessions((S.sessions as SessionMeta[]).filter((session) => session.key !== key));
	renderSessionList();
	if (!removedActive) return true;
	S.setActiveSessionKey(nextKey);
	if (opts.navigateIfActive && location.pathname.startsWith("/chats/")) navigate(sessionPath(nextKey));
	return true;
}

// ── New session button ──────────────────────────────────────
const newSessionBtn = S.$("newSessionBtn") as HTMLElement;
newSessionBtn.addEventListener("click", () => {
	const id = crypto.randomUUID
		? crypto.randomUUID()
		: ([1e7].toString() + -1e3 + -4e3 + -8e3 + -1e11).replace(/[018]/g, (c) =>
				(Number(c) ^ (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (Number(c) / 4)))).toString(16),
			);
	const key = `session:${id}`;
	const filterId = projectStore.projectFilterId.value;
	if (currentPrefix === "/chats") {
		switchSession(key, null, filterId || undefined);
	} else {
		navigate(sessionPath(key));
	}
});

export function isArchivableSession(session: SessionMeta): boolean {
	return (
		session.key !== "main" &&
		((session as SessionMeta & { activeChannel?: boolean }).activeChannel !== true || session.archived === true)
	);
}

function isClearableSession(session: SessionMeta): boolean {
	const isChannelSessionKey =
		session.key.startsWith("telegram:") ||
		session.key.startsWith("msteams:") ||
		session.key.startsWith("discord:") ||
		session.key.startsWith("slack:") ||
		session.key.startsWith("matrix:");
	return session.key !== "main" && !session.key.startsWith("cron:") && !isChannelSessionKey && !session.channelBinding;
}

export function clearAllSessions(): Promise<{ ok: boolean; skipped?: boolean; cancelled?: boolean }> {
	const allSessions = sessionStore.sessions.value;
	const count = allSessions.filter((session) => isClearableSession(session as unknown as SessionMeta)).length;
	if (count === 0) {
		return Promise.resolve({ ok: true, skipped: true });
	}
	return confirmDialog(
		`Delete ${count} session${count !== 1 ? "s" : ""}? Main, channel-bound, and cron sessions will be kept.`,
	).then((yes) => {
		if (!yes) return { ok: false, cancelled: true };
		return sendRpc("sessions.clear_all", {}).then((res) => {
			if (!res?.ok) return res;
			clearSessionHistory();
			// If the active session was deleted, switch to main.
			const active = sessionStore.getByKey(sessionStore.activeSessionKey.value);
			if (active && isClearableSession(active as unknown as SessionMeta)) {
				switchSession("main");
			}
			fetchSessions();
			return res;
		});
	});
}

// ── Re-render session list on project filter change ─────────
document.addEventListener("moltis:render-session-list", renderSessionList);
