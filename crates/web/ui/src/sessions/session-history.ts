// ── Session history: loading, caching, pagination ─────────────────

import { chatAddMsg } from "../chat-ui";
import * as S from "../state";
import {
	clearSessionHistory,
	getHistoryRevision,
	getSessionHistory,
	replaceSessionHistory,
	upsertSessionHistoryMessage,
} from "../stores/session-history-cache";
import { sessionStore } from "../stores/session-store";
import type { HistoryMessage } from "../types";

interface ChatParams {
	content?: unknown[];
	text?: string;
	_seq?: number | null;
}

interface HistoryPaginationState {
	hasMore: boolean;
	nextCursor: number | null;
	totalMessages: number | null;
	loadingOlder: boolean;
}

export interface HistoryPayload {
	historyCacheHit?: boolean;
	history?: HistoryMessage[];
	historyTruncated?: boolean;
	historyDroppedCount?: number;
	historyOmitted?: boolean;
	hasMore?: boolean;
	nextCursor?: number;
	totalMessages?: number;
}

/** HTTP error response from the sessions API. */
interface HttpErrorPayload {
	error?: string;
}

/** History message with optional created_at and seq fields for outgoing user messages. */
interface OutgoingUserMessage extends HistoryMessage {
	created_at?: number;
	seq?: number | null;
}

const SESSION_HISTORY_PAGE_LIMIT = 120;
const HISTORY_AUTOLOAD_THRESHOLD_PX = 120;
const sessionHistoryPaging = new Map<string, HistoryPaginationState>();
let historyScrollEl: HTMLElement | null = null;
let historyScrollRaf = 0;

function toValidHistoryIndex(value: unknown): number | null {
	if (value === null || value === undefined) return null;
	const idx = Number(value);
	if (!Number.isInteger(idx) || idx < 0) return null;
	return idx;
}

export function clearHistoryPaginationState(key?: string): void {
	if (key === undefined) {
		sessionHistoryPaging.clear();
		return;
	}
	if (!key) return;
	sessionHistoryPaging.delete(key);
}

export function setHistoryPaginationState(key: string, payload: HistoryPayload): void {
	if (!key) return;
	const hasMore = payload?.hasMore === true;
	const nextCursor = toValidHistoryIndex(payload?.nextCursor);
	const totalMessages = Number(payload?.totalMessages);
	sessionHistoryPaging.set(key, {
		hasMore: hasMore && nextCursor !== null,
		nextCursor: hasMore ? nextCursor : null,
		totalMessages: Number.isInteger(totalMessages) && totalMessages >= 0 ? totalMessages : null,
		loadingOlder: false,
	});
}

export function getHistoryPaginationState(key: string): HistoryPaginationState | null {
	return sessionHistoryPaging.get(key) || null;
}

export function isHistoryCacheComplete(key: string): boolean {
	const paging = getHistoryPaginationState(key);
	return !paging || paging.hasMore !== true;
}

export function historyIndexFromMessage(message: HistoryMessage | null | undefined): number | null {
	if (!(message && typeof message === "object")) return null;
	const idx = toValidHistoryIndex(message.historyIndex);
	if (idx !== null) return idx;
	return toValidHistoryIndex(message.messageIndex);
}

export function computeHistoryTailIndex(history: HistoryMessage[]): number {
	let max = -1;
	if (!Array.isArray(history)) return max;
	for (let i = 0; i < history.length; i += 1) {
		const indexed = historyIndexFromMessage(history[i]);
		if (indexed !== null) {
			if (indexed > max) max = indexed;
			continue;
		}
		if (i > max) max = i;
	}
	return max;
}

export function historyHasUnindexedMessages(history: HistoryMessage[]): boolean {
	if (!Array.isArray(history)) return false;
	for (const msg of history) {
		if (historyIndexFromMessage(msg) === null) return true;
	}
	return false;
}

function currentSessionTailIndex(key: string): number | null {
	const session = sessionStore.getByKey(key);
	if (session && typeof session.messageCount === "number" && session.messageCount > 0) {
		return session.messageCount - 1;
	}
	if (key === S.activeSessionKey && S.lastHistoryIndex >= 0) {
		return S.lastHistoryIndex + 1;
	}
	return null;
}

export function cacheSessionHistoryMessage(key: string, message: HistoryMessage, historyIndex?: number): void {
	upsertSessionHistoryMessage(key, message, historyIndex);
}

export function cacheOutgoingUserMessage(key: string, chatParams: ChatParams): void {
	if (!(key && chatParams)) return;
	const historyIndex = currentSessionTailIndex(key);
	const next: OutgoingUserMessage = {
		role: "user",
		content: (chatParams.content && Array.isArray(chatParams.content)
			? chatParams.content
			: chatParams.text || "") as string,
		created_at: Date.now(),
		seq: chatParams._seq || null,
	};
	if (historyIndex !== null) next.historyIndex = historyIndex;
	upsertSessionHistoryMessage(key, next, historyIndex ?? undefined);
}

export function clearSessionHistoryCache(key?: string): void {
	clearSessionHistory(key);
	clearHistoryPaginationState(key);
}

export async function fetchSessionHistoryViaHttp(
	key: string,
	options?: { cachedMessageCount?: number; cursor?: number; limit?: number },
): Promise<HistoryPayload> {
	const opts = options || {};
	const query = new URLSearchParams();
	if (Number.isInteger(opts.cachedMessageCount) && (opts.cachedMessageCount as number) >= 0) {
		query.set("cached_message_count", String(opts.cachedMessageCount));
	}
	if (Number.isInteger(opts.cursor) && (opts.cursor as number) >= 0) {
		query.set("cursor", String(opts.cursor));
	}
	if (Number.isInteger(opts.limit) && (opts.limit as number) > 0) {
		query.set("limit", String(opts.limit));
	}
	let url = `/api/sessions/${encodeURIComponent(key)}/history`;
	const qs = query.toString();
	if (qs) url += `?${qs}`;

	const response = await fetch(url, {
		headers: { Accept: "application/json" },
	});
	let payload: HistoryPayload | null = null;
	try {
		payload = await response.json();
	} catch {
		payload = null;
	}
	if (!response.ok) {
		const errMsg = (payload as HttpErrorPayload | null)?.error || `Failed to load session history (${response.status})`;
		throw new Error(errMsg);
	}
	return payload || {};
}

export function mergeHistoryPages(existingHistory: HistoryMessage[], olderHistory: HistoryMessage[]): HistoryMessage[] {
	const older = Array.isArray(olderHistory) ? olderHistory : [];
	const current = Array.isArray(existingHistory) ? existingHistory : [];
	if (older.length === 0) return current;
	if (current.length === 0) return older;

	const byIndex = new Map<number, HistoryMessage>();
	const ordered: HistoryMessage[] = [];
	const pushMessage = (msg: HistoryMessage): void => {
		const idx = historyIndexFromMessage(msg);
		if (idx === null) {
			ordered.push(msg);
			return;
		}
		if (!byIndex.has(idx)) {
			ordered.push(msg);
		}
		byIndex.set(idx, msg);
	};

	for (const olderMsg of older) pushMessage(olderMsg);
	for (const currentMsg of current) pushMessage(currentMsg);

	return ordered.map((msg) => {
		const idx = historyIndexFromMessage(msg);
		if (idx === null) return msg;
		return byIndex.get(idx) || msg;
	});
}

function canLoadOlderHistory(key: string): boolean {
	const paging = getHistoryPaginationState(key);
	if (!(paging?.hasMore && Number.isInteger(paging.nextCursor))) return false;
	if (paging.loadingOlder) return false;
	return true;
}

function maybeLoadOlderHistoryFromScroll(): void {
	if (!S.chatMsgBox) return;
	if (S.chatMsgBox.scrollTop > HISTORY_AUTOLOAD_THRESHOLD_PX) return;
	const key = sessionStore.activeSessionKey.value || S.activeSessionKey;
	if (!key) return;
	if (!canLoadOlderHistory(key)) return;
	void loadOlderHistoryPage(key);
}

function handleHistoryScroll(): void {
	if (historyScrollRaf) return;
	historyScrollRaf = requestAnimationFrame(() => {
		historyScrollRaf = 0;
		maybeLoadOlderHistoryFromScroll();
	});
}

export function ensureHistoryScrollBinding(): void {
	const nextEl = S.chatMsgBox;
	if (historyScrollEl === nextEl) return;
	if (historyScrollEl) {
		historyScrollEl.removeEventListener("scroll", handleHistoryScroll);
	}
	historyScrollEl = nextEl;
	if (!historyScrollEl) return;
	historyScrollEl.addEventListener("scroll", handleHistoryScroll, { passive: true });
}

async function loadOlderHistoryPage(key: string): Promise<void> {
	if (!canLoadOlderHistory(key)) return;
	const paging = getHistoryPaginationState(key);
	if (!paging) return;
	if (sessionStore.activeSessionKey.value !== key) return;

	const nextState = { ...paging, loadingOlder: true };
	sessionHistoryPaging.set(key, nextState);
	const loadedHistory = getSessionHistory(key) || [];
	const totalBefore = Number.isInteger(nextState.totalMessages) ? nextState.totalMessages! : loadedHistory.length;

	// Lazy import to avoid circular dependency
	const { renderHistory } = await import("./session-render");
	renderHistory(key, loadedHistory, null, null, totalBefore, true);

	const beforeHeight = S.chatMsgBox ? S.chatMsgBox.scrollHeight : 0;
	const beforeTop = S.chatMsgBox ? S.chatMsgBox.scrollTop : 0;

	try {
		const payload = await fetchSessionHistoryViaHttp(key, {
			cursor: nextState.nextCursor as number,
			limit: SESSION_HISTORY_PAGE_LIMIT,
		});
		if (sessionStore.activeSessionKey.value !== key) return;

		const older = Array.isArray(payload.history) ? payload.history : [];
		const current = getSessionHistory(key) || [];
		if (older.length > 0 && payload.historyCacheHit !== true) {
			replaceSessionHistory(key, mergeHistoryPages(current, older));
		}
		setHistoryPaginationState(key, payload);

		const merged = getSessionHistory(key) || [];
		const sessionEntry = sessionStore.getByKey(key);
		const totalCountHint = Number.isInteger(sessionEntry?.messageCount)
			? (sessionEntry?.messageCount as number)
			: Number(payload.totalMessages) || merged.length;
		renderHistory(key, merged, null, null, totalCountHint, true);

		if (S.chatMsgBox) {
			const afterHeight = S.chatMsgBox.scrollHeight;
			S.chatMsgBox.scrollTop = Math.max(0, beforeTop + (afterHeight - beforeHeight));
		}
	} catch {
		if (sessionStore.activeSessionKey.value !== key) return;
		const fallback = getSessionHistory(key) || [];
		const fallbackTotal = Number.isInteger(nextState.totalMessages) ? nextState.totalMessages! : fallback.length;
		sessionHistoryPaging.set(key, { ...nextState, loadingOlder: false });
		renderHistory(key, fallback, null, null, fallbackTotal, true);
		chatAddMsg("error", "Failed to load older messages");
	} finally {
		const latest = getHistoryPaginationState(key);
		if (latest) sessionHistoryPaging.set(key, { ...latest, loadingOlder: false });
		if (sessionStore.activeSessionKey.value === key) {
			maybeLoadOlderHistoryFromScroll();
		}
	}
}

export function shouldApplyServerHistory(
	key: string,
	serverHistory: HistoryMessage[],
	requestRevision: number,
): boolean {
	const current = getSessionHistory(key);
	if (!current) return true;
	const serverTail = computeHistoryTailIndex(serverHistory);
	const currentTail = computeHistoryTailIndex(current);
	if (serverTail > currentTail) return true;
	if (serverTail < currentTail) return false;
	const currentRevision = getHistoryRevision(key);
	if (currentRevision === requestRevision) return true;
	return !historyHasUnindexedMessages(current);
}

export function syncHistoryState(
	key: string,
	history: HistoryMessage[],
	historyTailIndex: number,
	totalCountHint: number | null,
): void {
	const loadedCount = Array.isArray(history) ? history.length : 0;
	const sessionEntry = sessionStore.getByKey(key);
	const legacy = (S.sessions as import("../types").SessionMeta[]).find((s) => s.key === key);
	const existingCount = Number.isInteger(sessionEntry?.messageCount) ? (sessionEntry?.messageCount as number) : 0;
	const legacyCount = Number.isInteger(legacy?.messageCount) ? (legacy?.messageCount as number) : 0;
	const hintedCount = Number.isInteger(totalCountHint) ? totalCountHint! : 0;
	const count = Math.max(loadedCount, existingCount, hintedCount, legacyCount);
	if (sessionEntry) {
		sessionEntry.syncCounts(count, count);
		sessionEntry.localUnread.value = false;
		sessionEntry.lastHistoryIndex.value = historyTailIndex;
	}
	if (legacy) {
		legacy.messageCount = count;
		legacy.lastSeenMessageCount = count;
		legacy._localUnread = false;
	}
	S.setLastHistoryIndex(historyTailIndex);
}

export { SESSION_HISTORY_PAGE_LIMIT };
