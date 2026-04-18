// ── Sessions: list, switch, status helpers ──────────────────

import {
	appendChannelFooter,
	appendReasoningDisclosure,
	chatAddMsg,
	chatAddMsgWithImages,
	highlightAndScroll,
	removeThinking,
	scrollChatToBottom,
	stripChannelPrefix,
	updateCommandInputUI,
	updateTokenBar,
} from "./chat-ui";
import { highlightCodeBlocks } from "./code-highlight";
import * as gon from "./gon";
import {
	formatAssistantTokenUsage,
	formatTokenSpeed,
	parseAgentsListPayload,
	renderAudioPlayer,
	renderDocument,
	renderMarkdown,
	renderScreenshot,
	sendRpc,
	tokenSpeedTone,
	toolCallSummary,
} from "./helpers";
import { attachMessageVoiceControl } from "./message-voice";
import { restoreNodeSelection } from "./nodes-selector";
import { updateSessionProjectSelect } from "./project-combo";
import { restoreReasoningFromModelId } from "./reasoning-toggle";
import { currentPrefix, navigate, sessionPath } from "./router";
import { settingsPath } from "./routes";
import { updateSandboxImageUI, updateSandboxUI } from "./sandbox";
import * as S from "./state";
import { modelStore } from "./stores/model-store";
import { projectStore } from "./stores/project-store";
import {
	clearSessionHistory,
	getHistoryRevision,
	getSessionHistory,
	replaceSessionHistory,
	upsertSessionHistoryMessage,
} from "./stores/session-history-cache";
import { insertSessionInOrder, Session, sessionStore } from "./stores/session-store";
import type { HistoryMessage, RpcResponse, SessionMeta } from "./types";
import { confirmDialog } from "./ui";

interface SearchContext {
	query: string;
	messageIndex: number;
}

interface ChatParams {
	content?: unknown[];
	text?: string;
	_seq?: number | null;
}

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

interface HistoryPaginationState {
	hasMore: boolean;
	nextCursor: number | null;
	totalMessages: number | null;
	loadingOlder: boolean;
}

interface HistoryPayload {
	historyCacheHit?: boolean;
	history?: HistoryMessage[];
	historyTruncated?: boolean;
	historyDroppedCount?: number;
	historyOmitted?: boolean;
	hasMore?: boolean;
	nextCursor?: number;
	totalMessages?: number;
}

interface SwitchPayload {
	entry?: SessionMeta;
	history?: HistoryMessage[];
	historyCacheHit?: boolean;
	historyTruncated?: boolean;
	historyDroppedCount?: number;
	historyOmitted?: boolean;
	replying?: boolean;
	thinkingText?: string;
	voicePending?: boolean;
	hasMore?: boolean;
	nextCursor?: number;
	totalMessages?: number;
}

interface ToolResultMsg extends HistoryMessage {
	tool_name?: string;
	arguments?: unknown;
	success?: boolean;
	result?: {
		stdout?: string;
		stderr?: string;
		exit_code?: number;
		screenshot?: string;
		document_ref?: string;
		filename?: string;
		mime_type?: string;
		size_bytes?: number;
	};
	error?: string;
	reasoning?: string;
}

interface AssistantMsg extends HistoryMessage {
	content?: string;
	model?: string;
	provider?: string;
	inputTokens?: number;
	outputTokens?: number;
	cacheReadTokens?: number;
	durationMs?: number;
	reasoning?: string;
	audio?: string;
	run_id?: string;
	historyIndex?: number;
	requestInputTokens?: number;
}

interface UserMsg extends Omit<HistoryMessage, "content"> {
	content?: string | unknown[];
	channel?: {
		channel_type?: string;
		username?: string;
		sender_name?: string;
		message_kind?: string;
	};
	audio?: string;
}

interface AgentInfo {
	id?: string;
	name?: string;
	emoji?: string;
}

const SESSION_PREVIEW_MAX_CHARS = 200;
const SESSION_LIST_PAGE_LIMIT = 40;
const SESSION_LIST_REFRESH_LIMIT_MAX = 200;
const SESSION_LIST_SCROLL_THRESHOLD = 220;
const HISTORY_AUTOLOAD_THRESHOLD_PX = 120;
const SESSION_HISTORY_PAGE_LIMIT = 120;
let switchRequestSeq = 0;
const latestSwitchRequestBySession = new Map<string, number>();
const sessionHistoryPaging = new Map<string, HistoryPaginationState>();
const sessionListPaging: SessionListPaging = {
	hasMore: false,
	nextCursor: null,
	total: null,
	loading: false,
};
let sessionListPendingRefresh = false;
let sessionListScrollEl: HTMLElement | null = null;
let sessionListScrollRaf = 0;
let historyScrollEl: HTMLElement | null = null;
let historyScrollRaf = 0;

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

/** Clear history for the currently active session and reset local UI state. */
export function clearActiveSession(): Promise<RpcResponse> {
	const prevHistoryIdx = S.lastHistoryIndex;
	const prevSeq = S.chatSeq;
	S.setLastHistoryIndex(-1);
	S.setChatSeq(0);
	return sendRpc("chat.clear", {}).then((res) => {
		if (res?.ok) {
			if (S.chatMsgBox) S.chatMsgBox.textContent = "";
			S.setSessionTokens({ input: 0, output: 0 });
			S.setSessionCurrentInputTokens(0);
			updateTokenBar();
			const activeKey = sessionStore.activeSessionKey.value || S.activeSessionKey;
			markSessionLocallyCleared(activeKey);
			clearSessionHistory(activeKey);
			clearHistoryPaginationState(activeKey);
			return res;
		}
		S.setLastHistoryIndex(prevHistoryIdx);
		S.setChatSeq(prevSeq);
		chatAddMsg("error", res?.error?.message || "Clear failed");
		return res;
	});
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

function toValidHistoryIndex(value: unknown): number | null {
	if (value === null || value === undefined) return null;
	const idx = Number(value);
	if (!Number.isInteger(idx) || idx < 0) return null;
	return idx;
}

function clearHistoryPaginationState(key?: string): void {
	if (key === undefined) {
		sessionHistoryPaging.clear();
		return;
	}
	if (!key) return;
	sessionHistoryPaging.delete(key);
}

function setHistoryPaginationState(key: string, payload: HistoryPayload): void {
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

function getHistoryPaginationState(key: string): HistoryPaginationState | null {
	return sessionHistoryPaging.get(key) || null;
}

function isHistoryCacheComplete(key: string): boolean {
	const paging = getHistoryPaginationState(key);
	return !paging || paging.hasMore !== true;
}

function historyIndexFromMessage(message: HistoryMessage | null | undefined): number | null {
	if (!(message && typeof message === "object")) return null;
	const idx = toValidHistoryIndex(message.historyIndex);
	if (idx !== null) return idx;
	return toValidHistoryIndex(message.messageIndex);
}

function computeHistoryTailIndex(history: HistoryMessage[]): number {
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

function historyHasUnindexedMessages(history: HistoryMessage[]): boolean {
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
	const next: HistoryMessage = {
		role: "user",
		content:
			chatParams.content && Array.isArray(chatParams.content)
				? (chatParams.content as unknown as string)
				: chatParams.text || "",
	};
	(next as Record<string, unknown>).created_at = Date.now();
	(next as Record<string, unknown>).seq = chatParams._seq || null;
	if (historyIndex !== null) next.historyIndex = historyIndex;
	upsertSessionHistoryMessage(key, next, historyIndex ?? undefined);
}

export function clearSessionHistoryCache(key?: string): void {
	clearSessionHistory(key);
	clearHistoryPaginationState(key);
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

// ── MCP toggle restore ──────────────────────────────────────
function restoreMcpToggle(mcpEnabled: boolean): void {
	const mcpBtn = S.$("mcpToggleBtn");
	const mcpLabel = S.$("mcpToggleLabel");
	if (mcpBtn) {
		mcpBtn.style.color = mcpEnabled ? "var(--ok)" : "var(--muted)";
		mcpBtn.style.borderColor = mcpEnabled ? "var(--ok)" : "var(--border)";
	}
	if (mcpLabel) mcpLabel.textContent = mcpEnabled ? "MCP" : "MCP off";
}

// ── Switch session ──────────────────────────────────────────

function restoreSessionState(entry: SessionMeta, projectId?: string): void {
	const effectiveProjectId = entry.projectId || projectId || "";
	projectStore.setActiveProjectId(effectiveProjectId);
	// Dual-write to state.js for backward compat
	S.setActiveProjectId(effectiveProjectId);
	localStorage.setItem("moltis-project", effectiveProjectId);
	updateSessionProjectSelect(effectiveProjectId);
	if (entry.model) {
		// Strip any @reasoning-* suffix and restore the toggle state
		const baseModelId = restoreReasoningFromModelId(entry.model);
		modelStore.select(baseModelId);
		// Dual-write to state.js for backward compat
		S.setSelectedModelId(baseModelId);
		localStorage.setItem("moltis-model", baseModelId);
		const found = modelStore.getById(baseModelId);
		if (S.modelComboLabel) S.modelComboLabel.textContent = found ? found.displayName || found.id : baseModelId;
	}
	updateSandboxUI(entry.sandbox_enabled !== false);
	updateSandboxImageUI(entry.sandbox_image || null);
	const sandboxRuntimeAvailable = ((S.sandboxInfo as Record<string, unknown> | null)?.backend || "none") !== "none";
	const effectiveSandboxRoute = entry.sandbox_enabled !== false && sandboxRuntimeAvailable;
	S.setSessionExecMode(effectiveSandboxRoute ? "sandbox" : "host");
	S.setSessionExecPromptSymbol(effectiveSandboxRoute || S.hostExecIsRoot ? "#" : "$");
	updateCommandInputUI();
	restoreMcpToggle(!entry.mcpDisabled);
	restoreNodeSelection(entry.node_id || null);
	updateChatSessionHeader();
}

/** Extract text and images from a multimodal content array. */
function parseMultimodalContent(blocks: unknown[]): { text: string; images: { dataUrl: string; name: string }[] } {
	let text = "";
	const images: { dataUrl: string; name: string }[] = [];
	for (const block of blocks as Array<{ type?: string; text?: string; image_url?: { url?: string } }>) {
		if (block.type === "text") {
			text = block.text || "";
		} else if (block.type === "image_url" && block.image_url?.url) {
			images.push({ dataUrl: block.image_url.url, name: "image" });
		}
	}
	return { text, images };
}

function renderHistoryUserMessage(msg: UserMsg): HTMLElement | null {
	let text = "";
	let images: { dataUrl: string; name: string }[] = [];
	if (Array.isArray(msg.content)) {
		const parsed = parseMultimodalContent(msg.content);
		text = msg.channel ? stripChannelPrefix(parsed.text) : parsed.text;
		images = parsed.images;
	} else {
		text = msg.channel ? stripChannelPrefix((msg.content as string) || "") : (msg.content as string) || "";
	}

	let el: HTMLElement | null;
	if (msg.audio) {
		el = chatAddMsg("user", "", true);
		if (el) {
			const filename = msg.audio.split("/").pop() || "";
			const audioSrc = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			renderAudioPlayer(el, audioSrc);
			if (text) {
				const textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				// Safe: renderMarkdown escapes user input before formatting tags.
				textWrap.insertAdjacentHTML("beforeend", renderMarkdown(text));
				el.appendChild(textWrap);
			}
			if (images.length > 0) {
				const thumbRow = document.createElement("div");
				thumbRow.className = "msg-image-row";
				for (const img of images) {
					const thumb = document.createElement("img");
					thumb.className = "msg-image-thumb";
					thumb.src = img.dataUrl;
					thumb.alt = img.name;
					thumbRow.appendChild(thumb);
				}
				el.appendChild(thumbRow);
			}
		}
	} else if (images.length > 0) {
		el = chatAddMsgWithImages("user", text ? renderMarkdown(text) : "", images);
	} else {
		el = chatAddMsg("user", renderMarkdown(text), true);
	}
	if (el && msg.channel) appendChannelFooter(el, msg.channel);
	return el;
}

function createModelFooter(msg: AssistantMsg): HTMLDivElement {
	const ft = document.createElement("div");
	ft.className = "msg-model-footer";
	let ftText = msg.provider ? `${msg.provider} / ${msg.model}` : msg.model || "";
	if (msg.inputTokens || msg.outputTokens) {
		ftText += ` \u00b7 ${formatAssistantTokenUsage(msg.inputTokens || 0, msg.outputTokens || 0, msg.cacheReadTokens || 0)}`;
	}
	const textSpan = document.createElement("span");
	textSpan.textContent = ftText;
	ft.appendChild(textSpan);

	const speedLabel = formatTokenSpeed(msg.outputTokens || 0, msg.durationMs || 0);
	if (speedLabel) {
		const speed = document.createElement("span");
		speed.className = "msg-token-speed";
		const tone = tokenSpeedTone(msg.outputTokens || 0, msg.durationMs || 0);
		if (tone) speed.classList.add(`msg-token-speed-${tone}`);
		speed.textContent = ` \u00b7 ${speedLabel}`;
		ft.appendChild(speed);
	}
	return ft;
}

function renderHistoryAssistantMessage(msg: AssistantMsg): HTMLElement | null {
	let el: HTMLElement | null;
	if (msg.audio) {
		// Voice response: render audio player first, then transcript text below.
		el = chatAddMsg("assistant", "", true);
		if (el) {
			const filename = msg.audio.split("/").pop() || "";
			const audioSrc = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			renderAudioPlayer(el, audioSrc);
			if (msg.content) {
				const textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				// Safe: renderMarkdown calls esc() first -- all user input is HTML-escaped.
				textWrap.insertAdjacentHTML("beforeend", renderMarkdown(msg.content));
				el.appendChild(textWrap);
			}
			if (msg.reasoning) {
				appendReasoningDisclosure(el, msg.reasoning);
			}
		}
	} else {
		el = chatAddMsg("assistant", renderMarkdown(msg.content || ""), true);
		if (el && msg.reasoning) {
			appendReasoningDisclosure(el, msg.reasoning);
		}
	}
	if (el && msg.model) {
		const footer = createModelFooter(msg);
		el.appendChild(footer);
		void attachMessageVoiceControl({
			messageEl: el,
			footerEl: footer,
			sessionKey: S.activeSessionKey,
			text: msg.content || "",
			runId: msg.run_id || undefined,
			messageIndex: msg.historyIndex,
			audioPath: msg.audio || undefined,
			audioWarning: undefined,
			forceAction: false,
			autoplayOnGenerate: true,
		});
	}
	if (msg.inputTokens || msg.outputTokens) {
		S.sessionTokens.input += msg.inputTokens || 0;
		S.sessionTokens.output += msg.outputTokens || 0;
	}
	if (msg.requestInputTokens !== undefined && msg.requestInputTokens !== null) {
		S.setSessionCurrentInputTokens(msg.requestInputTokens || 0);
	} else if (msg.inputTokens || msg.outputTokens) {
		S.setSessionCurrentInputTokens(msg.inputTokens || 0);
	}
	return el;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Sequential result field rendering
function renderHistoryToolResult(msg: ToolResultMsg): HTMLElement {
	const tpl = S.$<HTMLTemplateElement>("tpl-exec-card")!;
	const frag = tpl.content.cloneNode(true) as DocumentFragment;
	const card = frag.firstElementChild as HTMLElement;

	// Remove the "running..." status element -- this is a completed result.
	const statusEl = card.querySelector(".exec-status");
	if (statusEl) statusEl.remove();

	// Set command summary from arguments.
	const cmd = toolCallSummary(msg.tool_name, msg.arguments as Parameters<typeof toolCallSummary>[1]);
	(card.querySelector("[data-cmd]") as HTMLElement).textContent = ` ${cmd}`;

	// Set success/error CSS class (replace the default "running" class).
	card.className = `msg exec-card ${msg.success ? "exec-ok" : "exec-err"}`;

	// Append result output if present.
	if (msg.result) {
		const out = (msg.result.stdout || "").replace(/\n+$/, "");
		if (out) {
			const outEl = document.createElement("pre");
			outEl.className = "exec-output";
			outEl.textContent = out;
			card.appendChild(outEl);
		}
		const stderrText = (msg.result.stderr || "").replace(/\n+$/, "");
		if (stderrText) {
			const errEl = document.createElement("pre");
			errEl.className = "exec-output exec-stderr";
			errEl.textContent = stderrText;
			card.appendChild(errEl);
		}
		if (msg.result.exit_code !== undefined && msg.result.exit_code !== 0) {
			const codeEl = document.createElement("div");
			codeEl.className = "exec-exit";
			codeEl.textContent = `exit ${msg.result.exit_code}`;
			card.appendChild(codeEl);
		}
		// Render persisted screenshot from the media API.
		if (msg.result.screenshot && !msg.result.screenshot.startsWith("data:")) {
			const filename = msg.result.screenshot.split("/").pop() || "";
			const sessionKey = S.activeSessionKey || "main";
			const mediaSrc = `/api/sessions/${encodeURIComponent(sessionKey)}/media/${encodeURIComponent(filename)}`;
			renderScreenshot(card, mediaSrc);
		}
		// Render persisted document from the media API.
		if (msg.result.document_ref) {
			const docStoredName = msg.result.document_ref.split("/").pop() || "";
			const docDisplayName = msg.result.filename || docStoredName;
			const docSessionKey = S.activeSessionKey || "main";
			const docMediaSrc = `/api/sessions/${encodeURIComponent(docSessionKey)}/media/${encodeURIComponent(docStoredName)}`;
			renderDocument(card, docMediaSrc, docDisplayName, msg.result.mime_type, msg.result.size_bytes);
		}
	}

	// Append error detail if present.
	if (!msg.success && msg.error) {
		const errMsg = document.createElement("div");
		errMsg.className = "exec-error-detail";
		errMsg.textContent = msg.error;
		card.appendChild(errMsg);
	}

	// Append reasoning disclosure if this tool call carried thinking text.
	if (msg.reasoning) {
		appendReasoningDisclosure(card, msg.reasoning);
	}

	if (S.chatMsgBox) S.chatMsgBox.appendChild(card);
	return card;
}

export function appendLastMessageTimestamp(epochMs: number): void {
	if (!S.chatMsgBox) return;
	// Remove any previous last-message timestamp
	const old = S.chatMsgBox.querySelector(".msg-footer-time");
	if (old) old.remove();
	const lastMsg = S.chatMsgBox.lastElementChild;
	if (!lastMsg || lastMsg.classList.contains("user")) return;
	let footer = lastMsg.querySelector(".msg-model-footer") as HTMLElement | null;
	if (!footer) {
		footer = document.createElement("div");
		footer.className = "msg-model-footer";
		lastMsg.appendChild(footer);
	}
	const timeEl = document.createElement("time");
	timeEl.className = "msg-footer-time";
	timeEl.setAttribute("data-epoch-ms", String(epochMs));
	timeEl.textContent = new Date(epochMs).toISOString();
	// Wrap separator + time in a span so we can remove both easily
	const wrap = document.createElement("span");
	wrap.className = "msg-footer-time";
	wrap.appendChild(document.createTextNode(" \u00b7 "));
	wrap.appendChild(timeEl);
	footer.appendChild(wrap);
}

function makeThinkingDots(): HTMLElement {
	const tpl = S.$<HTMLTemplateElement>("tpl-thinking-dots")!;
	return (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
}

function postHistoryLoadActions(
	key: string,
	searchContext: SearchContext | null,
	msgEls: (HTMLElement | null)[],
	thinkingText: string | null,
	skipAutoScroll: boolean,
): void {
	sendRpc("chat.context", {}).then((ctxRes) => {
		if (ctxRes?.ok && ctxRes.payload) {
			const p = ctxRes.payload as Record<string, unknown>;
			if (p.tokenUsage) {
				const tu = p.tokenUsage as Record<string, number>;
				S.setSessionContextWindow(tu.contextWindow || 0);
				S.setSessionTokens({
					input: tu.inputTokens || 0,
					output: tu.outputTokens || 0,
				});
				S.setSessionCurrentInputTokens(tu.estimatedNextInputTokens || tu.currentInputTokens || tu.inputTokens || 0);
			}
			S.setSessionToolsEnabled((p as Record<string, unknown>).supportsTools !== false);
			const execution = (p.execution || {}) as Record<string, unknown>;
			const mode = execution.mode === "sandbox" ? "sandbox" : "host";
			const hostIsRoot = execution.hostIsRoot === true;
			let isRoot = execution.isRoot as boolean | undefined;
			if (typeof isRoot !== "boolean") {
				isRoot = mode === "sandbox" ? true : hostIsRoot;
			}
			S.setHostExecIsRoot(hostIsRoot);
			S.setSessionExecMode(mode);
			S.setSessionExecPromptSymbol(isRoot ? "#" : "$");
		}
		updateCommandInputUI();
		updateTokenBar();
	});
	updateTokenBar();

	if (!skipAutoScroll && searchContext?.query && S.chatMsgBox) {
		highlightAndScroll(msgEls, searchContext.messageIndex, searchContext.query);
	} else if (!skipAutoScroll) {
		scrollChatToBottom();
	}

	const session = sessionStore.getByKey(key);
	if (session?.replying.value && S.chatMsgBox) {
		removeThinking();
		const thinkEl = document.createElement("div");
		thinkEl.className = "msg assistant thinking";
		thinkEl.id = "thinkingIndicator";
		if (thinkingText) {
			const textEl = document.createElement("span");
			textEl.className = "thinking-text";
			textEl.textContent = thinkingText;
			thinkEl.appendChild(textEl);
		} else {
			thinkEl.appendChild(makeThinkingDots());
		}
		S.chatMsgBox.appendChild(thinkEl);
		if (!skipAutoScroll) scrollChatToBottom();
	}
}

function mergeHistoryPages(existingHistory: HistoryMessage[], olderHistory: HistoryMessage[]): HistoryMessage[] {
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

function ensureHistoryScrollBinding(): void {
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
			? (sessionEntry!.messageCount as number)
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

/** No-op -- the Preact SessionHeader component auto-updates from signals. */
export function updateChatSessionHeader(): void {
	// Retained for backward compat call sites; Preact handles rendering.
}

function renderWelcomeAgentPicker(
	card: HTMLElement,
	activeAgentId: string,
	onActiveAgentResolved: (agent: AgentInfo | null) => void,
): void {
	const container = card.querySelector("[data-welcome-agents]") as HTMLElement | null;
	if (!container) return;

	sendRpc("agents.list", {}).then((res) => {
		if (!card.isConnected) return;
		if (!res?.ok) {
			container.classList.add("hidden");
			return;
		}
		const parsed = parseAgentsListPayload(res.payload as Parameters<typeof parseAgentsListPayload>[0]);
		const agents = (parsed.agents || []) as AgentInfo[];
		const defaultId = (parsed.defaultId || "main") as string;
		const effectiveActive = activeAgentId || defaultId;

		container.textContent = "";
		container.classList.remove("hidden");
		container.classList.add("flex");

		let activeAgent: AgentInfo | null = null;
		for (const agent of agents) {
			if (!agent?.id) continue;
			if (agent.id === effectiveActive) activeAgent = agent;
			const chip = document.createElement("button");
			chip.type = "button";
			chip.className = agent.id === effectiveActive ? "provider-btn" : "provider-btn provider-btn-secondary";
			chip.style.fontSize = "0.7rem";
			chip.style.padding = "3px 8px";
			const labelPrefix = agent.emoji ? `${agent.emoji} ` : "";
			chip.textContent = `${labelPrefix}${agent.name || agent.id}`;
			chip.addEventListener("click", () => {
				const key = sessionStore.activeSessionKey.value || S.activeSessionKey || "main";
				sendRpc("agents.set_session", { session_key: key, agent_id: agent.id }).then((setRes) => {
					if (!setRes?.ok) return;
					const live = sessionStore.getByKey(key);
					if (live) {
						live.agent_id = agent.id || "";
						live.dataVersion.value++;
					}
					fetchSessions();
					const welcome = S.chatMsgBox?.querySelector("#welcomeCard");
					if (welcome) {
						welcome.remove();
						showWelcomeCard();
					}
				});
			});
			container.appendChild(chip);
		}

		const hatchBtn = document.createElement("button");
		hatchBtn.type = "button";
		hatchBtn.className = "provider-btn provider-btn-secondary";
		hatchBtn.style.fontSize = "0.7rem";
		hatchBtn.style.padding = "3px 8px";
		hatchBtn.textContent = "\u{1F95A} Hatch a new agent";
		hatchBtn.addEventListener("click", () => {
			navigate(settingsPath("agents/new"));
		});
		container.appendChild(hatchBtn);

		onActiveAgentResolved(activeAgent);
	});
}

function showWelcomeCard(): void {
	if (!S.chatMsgBox) return;
	S.chatMsgBox.classList.add("chat-messages-empty");

	if (modelStore.models.value.length === 0) {
		const noProvTpl = S.$<HTMLTemplateElement>("tpl-no-providers-card");
		if (!noProvTpl) return;
		const noProvCard = (noProvTpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
		S.chatMsgBox.appendChild(noProvCard);
		return;
	}

	const tpl = S.$<HTMLTemplateElement>("tpl-welcome-card");
	if (!tpl) return;
	const card = (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
	const identity = gon.get("identity");
	const userName = identity?.user_name;
	const botName = identity?.name || "moltis";
	const botEmoji = identity?.emoji || "";

	const greetingEl = card.querySelector("[data-welcome-greeting]") as HTMLElement | null;
	if (greetingEl) greetingEl.textContent = userName ? `Hello, ${userName}!` : "Hello!";
	const emojiEl = card.querySelector("[data-welcome-emoji]") as HTMLElement | null;
	if (emojiEl) emojiEl.textContent = botEmoji;
	const nameEl = card.querySelector("[data-welcome-bot-name]") as HTMLElement | null;
	if (nameEl) nameEl.textContent = botName;
	const activeAgentId = (sessionStore.activeSession.value as unknown as SessionMeta)?.agent_id || "main";
	renderWelcomeAgentPicker(card, activeAgentId, (activeAgent) => {
		if (!activeAgent) return;
		if (emojiEl) emojiEl.textContent = activeAgent.emoji || "";
		if (nameEl) nameEl.textContent = activeAgent.name || botName;
	});

	S.chatMsgBox.appendChild(card);
}

export function refreshWelcomeCardIfNeeded(): void {
	if (!S.chatMsgBox) return;
	const welcomeCard = S.chatMsgBox.querySelector("#welcomeCard");
	const noProvCard = S.chatMsgBox.querySelector("#noProvidersCard");
	const hasModels = modelStore.models.value.length > 0;

	// Wrong variant showing -- swap it
	if (hasModels && noProvCard) {
		noProvCard.remove();
		showWelcomeCard();
	} else if (!hasModels && welcomeCard) {
		welcomeCard.remove();
		showWelcomeCard();
	}
}

function ensureSessionInClientStore(key: string, entry: SessionMeta, projectId?: string): unknown {
	const existing = sessionStore.getByKey(key);
	if (existing) return existing;

	const created: SessionMeta = { ...entry, key };
	if (projectId && !created.projectId) created.projectId = projectId;
	const createdSession = sessionStore.upsert(created);

	// Keep state.js mirror in sync for legacy call sites.
	const inLegacy = (S.sessions as SessionMeta[]).some((s) => s.key === key);
	if (!inLegacy) {
		S.setSessions(insertSessionInOrder(S.sessions as Session[], new Session(created)));
	}
	return createdSession;
}

function showSessionLoadIndicator(): void {
	if (!S.chatMsgBox) return;
	hideSessionLoadIndicator();
	const loading = document.createElement("div");
	loading.id = "sessionLoadIndicator";
	loading.className = "msg assistant thinking session-loading";
	loading.appendChild(makeThinkingDots());
	const label = document.createElement("span");
	label.className = "session-loading-label";
	label.textContent = "Loading session\u2026";
	loading.appendChild(label);
	S.chatMsgBox.appendChild(loading);
}

function hideSessionLoadIndicator(): void {
	const loading = document.getElementById("sessionLoadIndicator");
	if (loading) loading.remove();
}

function startSwitchRequest(key: string): number {
	switchRequestSeq += 1;
	latestSwitchRequestBySession.set(key, switchRequestSeq);
	return switchRequestSeq;
}

function isLatestSwitchRequest(key: string, requestId: number): boolean {
	return latestSwitchRequestBySession.get(key) === requestId;
}

function startSessionRefresh(key: string, blockRealtimeEvents: boolean): void {
	sessionStore.refreshInProgressKey.value = key;
	sessionStore.switchInProgress.value = !!blockRealtimeEvents;
	S.setSessionSwitchInProgress(!!blockRealtimeEvents);
}

function finishSessionRefresh(key: string): void {
	if (sessionStore.refreshInProgressKey.value !== key) return;
	sessionStore.refreshInProgressKey.value = "";
	sessionStore.switchInProgress.value = false;
	S.setSessionSwitchInProgress(false);
}

function resetSwitchViewState(): void {
	hideSessionLoadIndicator();
	if (S.chatMsgBox) S.chatMsgBox.textContent = "";
	const tray = document.getElementById("queuedMessages");
	if (tray) {
		while (tray.firstChild) tray.removeChild(tray.firstChild);
		tray.classList.add("hidden");
	}
	S.setStreamEl(null);
	S.setStreamText("");
	S.setLastToolOutput("");
	S.setVoicePending(false);
	S.setLastHistoryIndex(-1);
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionCurrentInputTokens(0);
	S.setSessionContextWindow(0);
	updateTokenBar();
}

function syncHistoryState(
	key: string,
	history: HistoryMessage[],
	historyTailIndex: number,
	totalCountHint: number | null,
): void {
	const loadedCount = Array.isArray(history) ? history.length : 0;
	const sessionEntry = sessionStore.getByKey(key);
	const legacy = (S.sessions as SessionMeta[]).find((s) => s.key === key);
	const existingCount = Number.isInteger(sessionEntry?.messageCount) ? (sessionEntry!.messageCount as number) : 0;
	const legacyCount = Number.isInteger(legacy?.messageCount) ? (legacy!.messageCount as number) : 0;
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

function renderHistory(
	key: string,
	history: HistoryMessage[],
	searchContext: SearchContext | null,
	thinkingText: string | null,
	totalCountHint: number | null,
	skipAutoScroll: boolean,
): void {
	ensureHistoryScrollBinding();
	hideSessionLoadIndicator();
	if (S.chatMsgBox) {
		S.chatMsgBox.classList.remove("chat-messages-empty");
		S.chatMsgBox.textContent = "";
	}
	const msgEls: (HTMLElement | null)[] = [];
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionCurrentInputTokens(0);
	S.setChatBatchLoading(true);
	history.forEach((msg) => {
		if (msg.role === "user") {
			msgEls.push(renderHistoryUserMessage(msg as UserMsg));
		} else if (msg.role === "assistant") {
			msgEls.push(renderHistoryAssistantMessage(msg as AssistantMsg));
		} else if (msg.role === "notice") {
			msgEls.push(chatAddMsg("system", renderMarkdown(msg.content || ""), true));
		} else if (msg.role === "tool_result") {
			msgEls.push(renderHistoryToolResult(msg as ToolResultMsg));
		} else {
			msgEls.push(null);
		}
	});
	S.setChatBatchLoading(false);
	// Syntax-highlight all code blocks in the rendered history.
	if (S.chatMsgBox) highlightCodeBlocks(S.chatMsgBox);
	const historyTailIndex = computeHistoryTailIndex(history);
	syncHistoryState(key, history, historyTailIndex, totalCountHint);

	// Resume chatSeq from the highest user message seq in history
	// so the counter continues from where it left off after reload.
	let maxSeq = 0;
	for (const hm of history) {
		if (hm.role === "user" && ((hm as Record<string, unknown>).seq as number) > maxSeq) {
			maxSeq = (hm as Record<string, unknown>).seq as number;
		}
	}
	S.setChatSeq(maxSeq);
	if (history.length === 0) {
		showWelcomeCard();
	} else {
		const lastMsg = history[history.length - 1];
		const ts = (lastMsg as Record<string, unknown>).created_at as number | undefined;
		if (ts) appendLastMessageTimestamp(ts);
	}
	postHistoryLoadActions(key, searchContext, msgEls, thinkingText, skipAutoScroll === true);
}

function shouldApplyServerHistory(key: string, serverHistory: HistoryMessage[], requestRevision: number): boolean {
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

function applyReplyingStateFromSwitchPayload(key: string, payload: SwitchPayload): void {
	const replying = payload.replying === true;
	setSessionReplying(key, replying);
	const voiceSession = sessionStore.getByKey(key);
	if (replying && payload.voicePending) {
		S.setVoicePending(true);
		if (voiceSession) voiceSession.voicePending.value = true;
	} else {
		S.setVoicePending(false);
		if (voiceSession) voiceSession.voicePending.value = false;
	}
	if (!replying && key === sessionStore.activeSessionKey.value) {
		removeThinking();
	}
}

async function fetchSessionHistoryViaHttp(
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
		const errMsg =
			(payload as Record<string, unknown> | null)?.error || `Failed to load session history (${response.status})`;
		throw new Error(errMsg as string);
	}
	return payload || {};
}

export function switchSession(key: string, searchContext?: SearchContext | null, projectId?: string): void {
	sessionStore.setActive(key);
	// Dual-write to state.js for backward compat
	S.setActiveSessionKey(key);
	localStorage.setItem("moltis-session", key);
	history.replaceState(null, "", sessionPath(key));
	resetSwitchViewState();
	const cachedEntry = sessionStore.getByKey(key);
	if (cachedEntry) {
		restoreSessionState(cachedEntry as unknown as SessionMeta, projectId);
	}
	// Preact SessionList auto-rerenders active/unread from signals.

	const switchReqId = startSwitchRequest(key);
	const switchParams: Record<string, unknown> = { key };
	if (projectId) switchParams.project_id = projectId;
	// Keep WebSocket for live state only; history comes from HTTP (gzip-friendly).
	switchParams.include_history = false;
	const cachedHistory = getSessionHistory(key);
	const hasCache = Array.isArray(cachedHistory);
	const cacheRevisionAtRequest = getHistoryRevision(key);
	const cacheComplete = hasCache && isHistoryCacheComplete(key);
	const cachedHistoryCount = cacheComplete
		? Number.isInteger(cachedEntry?.messageCount)
			? (cachedEntry!.messageCount as number)
			: cachedHistory?.length
		: null;
	startSessionRefresh(key, !hasCache);
	if (hasCache) {
		renderHistory(key, cachedHistory!, searchContext || null, null, cachedHistoryCount, false);
	} else {
		showSessionLoadIndicator();
	}

	sendRpc("sessions.switch", switchParams)
		.then(async (res) => {
			if (!isLatestSwitchRequest(key, switchReqId)) return;
			let stillActive = sessionStore.activeSessionKey.value === key;
			if (!(res?.ok && res.payload)) {
				if (stillActive && !hasCache) {
					hideSessionLoadIndicator();
					chatAddMsg("error", res?.error?.message || "Failed to load session");
				}
				finishSessionRefresh(key);
				if (stillActive && S.chatInput) S.chatInput.focus();
				return;
			}

			const switchPayload = res.payload as SwitchPayload;
			const entry = switchPayload.entry || ({} as SessionMeta);
			ensureSessionInClientStore(key, entry, projectId);
			const pagingBefore = getHistoryPaginationState(key);
			const pagingBeforeHasMore = pagingBefore?.hasMore === true;
			const pagingBeforeCursor = Number.isInteger(pagingBefore?.nextCursor) ? pagingBefore?.nextCursor : null;
			let historyPayload: HistoryPayload = {
				historyCacheHit: switchPayload.historyCacheHit === true,
				history: Array.isArray(switchPayload.history) ? switchPayload.history : [],
				historyTruncated: switchPayload.historyTruncated === true,
				historyDroppedCount: Number(switchPayload.historyDroppedCount) || 0,
			};
			if (switchPayload.historyOmitted === true) {
				try {
					historyPayload = await fetchSessionHistoryViaHttp(key, {
						cachedMessageCount: cachedHistoryCount ?? undefined,
						limit: SESSION_HISTORY_PAGE_LIMIT,
					});
				} catch (error) {
					if (!isLatestSwitchRequest(key, switchReqId)) return;
					stillActive = sessionStore.activeSessionKey.value === key;
					if (stillActive && !hasCache) {
						hideSessionLoadIndicator();
						chatAddMsg("error", (error as Error)?.message || "Failed to load session history");
					}
					finishSessionRefresh(key);
					if (stillActive && S.chatInput) S.chatInput.focus();
					return;
				}
				if (!isLatestSwitchRequest(key, switchReqId)) return;
				stillActive = sessionStore.activeSessionKey.value === key;
			}
			setHistoryPaginationState(key, historyPayload);
			const pagingAfter = getHistoryPaginationState(key);
			const pagingAfterHasMore = pagingAfter?.hasMore === true;
			const pagingAfterCursor = Number.isInteger(pagingAfter?.nextCursor) ? pagingAfter?.nextCursor : null;
			const paginationChanged = pagingBeforeHasMore !== pagingAfterHasMore || pagingBeforeCursor !== pagingAfterCursor;

			const cacheHit = historyPayload.historyCacheHit === true;
			const serverHistory = Array.isArray(historyPayload.history) ? historyPayload.history : [];
			let appliedServerHistory = false;
			if (!cacheHit && shouldApplyServerHistory(key, serverHistory, cacheRevisionAtRequest)) {
				replaceSessionHistory(key, serverHistory);
				appliedServerHistory = true;
			}
			const resolvedHistory = getSessionHistory(key) || serverHistory;
			if (stillActive) {
				restoreSessionState(entry, projectId);
				applyReplyingStateFromSwitchPayload(key, switchPayload);
				const thinkingText = switchPayload.replying ? switchPayload.thinkingText || null : null;
				const totalCountHint = Number.isInteger(entry.messageCount)
					? entry.messageCount!
					: Number(historyPayload.totalMessages) || resolvedHistory.length;
				const shouldRerender = !hasCache || Boolean(searchContext?.query) || appliedServerHistory || paginationChanged;
				if (shouldRerender) {
					renderHistory(key, resolvedHistory, searchContext || null, thinkingText, totalCountHint, false);
				} else {
					postHistoryLoadActions(key, searchContext || null, [], thinkingText, false);
				}
				if (appliedServerHistory && historyPayload.historyTruncated === true) {
					const dropped = Number(historyPayload.historyDroppedCount) || 0;
					chatAddMsg(
						"system",
						`Loaded the most recent messages for performance (${dropped} older message${dropped === 1 ? "" : "s"} omitted).`,
					);
				}
				if (appliedServerHistory && historyPayload.hasMore === true) {
					const total = Number(historyPayload.totalMessages) || resolvedHistory.length;
					chatAddMsg(
						"system",
						`Loaded recent history (${resolvedHistory.length} of ${total} messages) for faster loading.`,
					);
				}
				if (S.chatInput) S.chatInput.focus();
			}
			finishSessionRefresh(key);
		})
		.catch(() => {
			if (!isLatestSwitchRequest(key, switchReqId)) return;
			const stillActive = sessionStore.activeSessionKey.value === key;
			if (stillActive && !hasCache) {
				hideSessionLoadIndicator();
				chatAddMsg("error", "Failed to load session");
			}
			finishSessionRefresh(key);
			if (stillActive && S.chatInput) S.chatInput.focus();
		});
}
