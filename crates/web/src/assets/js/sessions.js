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
} from "./chat-ui.js";
import { highlightCodeBlocks } from "./code-highlight.js";
import * as gon from "./gon.js";
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
} from "./helpers.js";
import { attachMessageVoiceControl } from "./message-voice.js";
import { restoreNodeSelection } from "./nodes-selector.js";
import { updateSessionProjectSelect } from "./project-combo.js";
import { currentPrefix, navigate, sessionPath } from "./router.js";
import { settingsPath } from "./routes.js";
import { updateSandboxImageUI, updateSandboxUI } from "./sandbox.js";
import * as S from "./state.js";
import { modelStore } from "./stores/model-store.js";
import { projectStore } from "./stores/project-store.js";
import {
	clearSessionHistory,
	getHistoryRevision,
	getSessionHistory,
	replaceSessionHistory,
	upsertSessionHistoryMessage,
} from "./stores/session-history-cache.js";
import { insertSessionInOrder, sessionStore } from "./stores/session-store.js";
import { confirmDialog } from "./ui.js";

var SESSION_PREVIEW_MAX_CHARS = 200;
var SESSION_LIST_PAGE_LIMIT = 40;
var SESSION_LIST_REFRESH_LIMIT_MAX = 200;
var SESSION_LIST_SCROLL_THRESHOLD = 220;
var HISTORY_AUTOLOAD_THRESHOLD_PX = 120;
var SESSION_HISTORY_PAGE_LIMIT = 120;
var switchRequestSeq = 0;
var latestSwitchRequestBySession = new Map();
var sessionHistoryPaging = new Map();
var sessionListPaging = {
	hasMore: false,
	nextCursor: null,
	total: null,
	loading: false,
};
var sessionListPendingRefresh = false;
var sessionListScrollEl = null;
var sessionListScrollRaf = 0;
var historyScrollEl = null;
var historyScrollRaf = 0;

function truncateSessionPreview(text) {
	var trimmed = (text || "").trim();
	if (!trimmed) return "";
	var chars = Array.from(trimmed);
	if (chars.length <= SESSION_PREVIEW_MAX_CHARS) return trimmed;
	return `${chars.slice(0, SESSION_PREVIEW_MAX_CHARS).join("")}…`;
}

// ── Fetch & render ──────────────────────────────────────────

export function fetchSessions() {
	ensureSessionListScrollBinding();
	if (sessionListPaging.loading) {
		sessionListPendingRefresh = true;
		return;
	}

	sessionListPaging.loading = true;
	var loadedCount = Array.isArray(S.sessions) ? S.sessions.length : 0;
	var refreshLimit = Math.max(
		SESSION_LIST_PAGE_LIMIT,
		Math.min(
			Number.isInteger(loadedCount) && loadedCount > 0 ? loadedCount : SESSION_LIST_PAGE_LIMIT,
			SESSION_LIST_REFRESH_LIMIT_MAX,
		),
	);

	void fetchSessionListPage({ limit: refreshLimit })
		.then((page) => {
			var merged = mergeSessionListPage(S.sessions, page.sessions, false);
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

function toValidCursor(value) {
	var parsed = Number(value);
	if (!Number.isInteger(parsed) || parsed < 0) return null;
	return parsed;
}

function parseSessionListPayload(payload) {
	if (Array.isArray(payload)) {
		return {
			sessions: payload,
			hasMore: false,
			nextCursor: null,
			total: payload.length,
		};
	}

	var list = Array.isArray(payload?.sessions) ? payload.sessions : [];
	var nextCursor = toValidCursor(payload?.nextCursor);
	var hasMore = payload?.hasMore === true && nextCursor !== null;
	var total = Number(payload?.total);
	return {
		sessions: list,
		hasMore: hasMore,
		nextCursor: hasMore ? nextCursor : null,
		total: Number.isInteger(total) && total >= 0 ? total : null,
	};
}

function mergeSessionListPage(existingSessions, incomingSessions, append) {
	var existing = Array.isArray(existingSessions) ? existingSessions : [];
	var incoming = Array.isArray(incomingSessions) ? incomingSessions : [];

	var oldByKey = {};
	for (var old of existing) {
		if (!old?.key) continue;
		oldByKey[old.key] = old;
	}

	function withLocalFlags(session) {
		if (!(session && session.key)) return session;
		var prev = oldByKey[session.key];
		if (!prev) return session;
		var merged = { ...session };
		if (prev._localUnread) merged._localUnread = true;
		if (prev._replying) merged._replying = true;
		if (prev._activeRunId) merged._activeRunId = prev._activeRunId;
		return merged;
	}

	if (!append) {
		return incoming.map((session) => withLocalFlags(session));
	}

	var result = existing.slice();
	var indexByKey = {};
	for (var i = 0; i < result.length; i += 1) {
		var key = result[i]?.key;
		if (!key) continue;
		indexByKey[key] = i;
	}

	for (var session of incoming) {
		if (!(session && session.key)) continue;
		var next = withLocalFlags(session);
		var idx = indexByKey[session.key];
		if (Number.isInteger(idx)) {
			result[idx] = { ...result[idx], ...next };
			continue;
		}
		indexByKey[session.key] = result.length;
		result.push(next);
	}

	return result;
}

function applySessionList(sessions) {
	// Update session store (source of truth) — version guard
	// inside Session.update() prevents stale data from overwriting.
	sessionStore.setAll(sessions);
	// Dual-write to state.js for backward compat
	S.setSessions(sessions);
	renderSessionList();
	updateChatSessionHeader();
}

function applySessionListPaging(page) {
	sessionListPaging.hasMore = page.hasMore === true && Number.isInteger(page.nextCursor);
	sessionListPaging.nextCursor = sessionListPaging.hasMore ? page.nextCursor : null;
	sessionListPaging.total = Number.isInteger(page.total) ? page.total : null;
}

async function fetchSessionListPage(options) {
	var opts = options || {};
	var query = new URLSearchParams();
	if (Number.isInteger(opts.cursor) && opts.cursor >= 0) {
		query.set("cursor", String(opts.cursor));
	}
	if (Number.isInteger(opts.limit) && opts.limit > 0) {
		query.set("limit", String(opts.limit));
	}

	var url = "/api/sessions";
	var qs = query.toString();
	if (qs) url += `?${qs}`;

	var response = await fetch(url, {
		headers: { Accept: "application/json" },
	});
	var payload = null;
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

function shouldLoadMoreSessions() {
	var el = S.$("sessionList");
	if (!el) return false;
	if (el.clientHeight <= 0) return false;
	if (sessionListPaging.loading) return false;
	if (!(sessionListPaging.hasMore && Number.isInteger(sessionListPaging.nextCursor))) return false;
	var distance = el.scrollHeight - (el.scrollTop + el.clientHeight);
	return distance <= SESSION_LIST_SCROLL_THRESHOLD;
}

async function loadMoreSessionsPage() {
	if (!shouldLoadMoreSessions()) return;
	sessionListPaging.loading = true;
	try {
		var page = await fetchSessionListPage({
			cursor: sessionListPaging.nextCursor,
			limit: SESSION_LIST_PAGE_LIMIT,
		});
		var merged = mergeSessionListPage(S.sessions, page.sessions, true);
		applySessionList(merged);
		if (page.sessions.length === 0) {
			applySessionListPaging({
				hasMore: false,
				nextCursor: null,
				total: page.total,
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

function maybeLoadMoreSessionsFromScroll() {
	if (!shouldLoadMoreSessions()) return;
	void loadMoreSessionsPage();
}

function handleSessionListScroll() {
	if (sessionListScrollRaf) return;
	sessionListScrollRaf = requestAnimationFrame(() => {
		sessionListScrollRaf = 0;
		maybeLoadMoreSessionsFromScroll();
	});
}

function ensureSessionListScrollBinding() {
	var nextEl = S.$("sessionList");
	if (sessionListScrollEl === nextEl) return;
	if (sessionListScrollEl) {
		sessionListScrollEl.removeEventListener("scroll", handleSessionListScroll);
	}
	sessionListScrollEl = nextEl;
	if (!sessionListScrollEl) return;
	sessionListScrollEl.addEventListener("scroll", handleSessionListScroll, { passive: true });
}

export function markSessionLocallyCleared(key) {
	if (!key) return;
	var now = Date.now();

	var session = sessionStore.getByKey(key);
	if (session) {
		session.syncCounts(0, 0);
		session.preview = "";
		session.updatedAt = now;
		session.replying.value = false;
		session.activeRunId.value = null;
		session.lastHistoryIndex.value = -1;
		var localVersion = Number.isInteger(session.version) ? session.version : 0;
		session.version = localVersion + 1;
		session.dataVersion.value++;
	}

	var legacy = S.sessions.find((s) => s.key === key);
	if (legacy) {
		legacy.messageCount = 0;
		legacy.lastSeenMessageCount = 0;
		legacy.preview = "";
		legacy.updatedAt = now;
		legacy._localUnread = false;
		legacy._replying = false;
		legacy._activeRunId = null;
		var legacyVersion = Number.isInteger(legacy.version) ? legacy.version : 0;
		legacy.version = legacyVersion + 1;
	}
}

/** Clear history for the currently active session and reset local UI state. */
export function clearActiveSession() {
	var prevHistoryIdx = S.lastHistoryIndex;
	var prevSeq = S.chatSeq;
	S.setLastHistoryIndex(-1);
	S.setChatSeq(0);
	return sendRpc("chat.clear", {}).then((res) => {
		if (res?.ok) {
			if (S.chatMsgBox) S.chatMsgBox.textContent = "";
			S.setSessionTokens({ input: 0, output: 0 });
			S.setSessionCurrentInputTokens(0);
			updateTokenBar();
			var activeKey = sessionStore.activeSessionKey.value || S.activeSessionKey;
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

export function renderSessionList() {
	ensureSessionListScrollBinding();
	maybeLoadMoreSessionsFromScroll();
}

// ── Status helpers ──────────────────────────────────────────

export function setSessionReplying(key, replying) {
	// Update store signal — Preact SessionList re-renders automatically.
	var session = sessionStore.getByKey(key);
	if (session) session.replying.value = replying;
	// Dual-write: update plain S.sessions object
	var entry = S.sessions.find((s) => s.key === key);
	if (entry) entry._replying = replying;
}

export function setSessionActiveRunId(key, runId) {
	var session = sessionStore.getByKey(key);
	if (session) session.activeRunId.value = runId || null;
	var entry = S.sessions.find((s) => s.key === key);
	if (entry) entry._activeRunId = runId || null;
}

export function setSessionUnread(key, unread) {
	// Update store signal — Preact SessionList re-renders automatically.
	var session = sessionStore.getByKey(key);
	if (session) session.localUnread.value = unread;
	// Dual-write: update plain S.sessions object
	var entry = S.sessions.find((s) => s.key === key);
	if (entry) entry._localUnread = unread;
}

export function bumpSessionCount(key, increment) {
	// Update store — bumpCount bumps dataVersion for automatic re-render.
	var session = sessionStore.getByKey(key);
	if (session) {
		session.bumpCount(increment);
	}

	// Dual-write: update the underlying S.sessions data.
	var entry = S.sessions.find((s) => s.key === key);
	if (entry) {
		entry.messageCount = (entry.messageCount || 0) + increment;
		if (key === S.activeSessionKey) {
			entry.lastSeenMessageCount = entry.messageCount;
		}
	}
}

/** Set first-message preview optimistically so sidebar updates without reload. */
export function seedSessionPreviewFromUserText(key, text) {
	var preview = truncateSessionPreview(text);
	if (!preview) return;
	var now = Date.now();

	var session = sessionStore.getByKey(key);
	if (session && !session.preview) {
		session.preview = preview;
		session.updatedAt = now;
		session.dataVersion.value++;
	}

	var entry = S.sessions.find((s) => s.key === key);
	if (entry && !entry.preview) {
		entry.preview = preview;
		entry.updatedAt = now;
	}
}

function toValidHistoryIndex(value) {
	if (value === null || value === undefined) return null;
	var idx = Number(value);
	if (!Number.isInteger(idx) || idx < 0) return null;
	return idx;
}

function clearHistoryPaginationState(key) {
	if (key === undefined) {
		sessionHistoryPaging.clear();
		return;
	}
	if (!key) return;
	sessionHistoryPaging.delete(key);
}

function setHistoryPaginationState(key, payload) {
	if (!key) return;
	var hasMore = payload?.hasMore === true;
	var nextCursor = toValidHistoryIndex(payload?.nextCursor);
	var totalMessages = Number(payload?.totalMessages);
	sessionHistoryPaging.set(key, {
		hasMore: hasMore && nextCursor !== null,
		nextCursor: hasMore ? nextCursor : null,
		totalMessages: Number.isInteger(totalMessages) && totalMessages >= 0 ? totalMessages : null,
		loadingOlder: false,
	});
}

function getHistoryPaginationState(key) {
	return sessionHistoryPaging.get(key) || null;
}

function isHistoryCacheComplete(key) {
	var paging = getHistoryPaginationState(key);
	return !paging || paging.hasMore !== true;
}

function historyIndexFromMessage(message) {
	if (!(message && typeof message === "object")) return null;
	var idx = toValidHistoryIndex(message.historyIndex);
	if (idx !== null) return idx;
	return toValidHistoryIndex(message.messageIndex);
}

function computeHistoryTailIndex(history) {
	var max = -1;
	if (!Array.isArray(history)) return max;
	for (var i = 0; i < history.length; i += 1) {
		var indexed = historyIndexFromMessage(history[i]);
		if (indexed !== null) {
			if (indexed > max) max = indexed;
			continue;
		}
		if (i > max) max = i;
	}
	return max;
}

function historyHasUnindexedMessages(history) {
	if (!Array.isArray(history)) return false;
	for (var msg of history) {
		if (historyIndexFromMessage(msg) === null) return true;
	}
	return false;
}

function currentSessionTailIndex(key) {
	var session = sessionStore.getByKey(key);
	if (session && typeof session.messageCount === "number" && session.messageCount > 0) {
		return session.messageCount - 1;
	}
	if (key === S.activeSessionKey && S.lastHistoryIndex >= 0) {
		return S.lastHistoryIndex + 1;
	}
	return null;
}

export function cacheSessionHistoryMessage(key, message, historyIndex) {
	return upsertSessionHistoryMessage(key, message, historyIndex);
}

export function cacheOutgoingUserMessage(key, chatParams) {
	if (!(key && chatParams)) return;
	var historyIndex = currentSessionTailIndex(key);
	var next = {
		role: "user",
		content: chatParams.content && Array.isArray(chatParams.content) ? chatParams.content : chatParams.text || "",
		created_at: Date.now(),
		seq: chatParams._seq || null,
	};
	if (historyIndex !== null) next.historyIndex = historyIndex;
	upsertSessionHistoryMessage(key, next, historyIndex);
}

export function clearSessionHistoryCache(key) {
	clearSessionHistory(key);
	clearHistoryPaginationState(key);
}

// ── New session button ──────────────────────────────────────
var newSessionBtn = S.$("newSessionBtn");
newSessionBtn.addEventListener("click", () => {
	var id = crypto.randomUUID
		? crypto.randomUUID()
		: ([1e7] + -1e3 + -4e3 + -8e3 + -1e11).replace(/[018]/g, (c) =>
				(c ^ (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (c / 4)))).toString(16),
			);
	var key = `session:${id}`;
	var filterId = projectStore.projectFilterId.value;
	if (currentPrefix === "/chats") {
		switchSession(key, null, filterId || undefined);
	} else {
		navigate(sessionPath(key));
	}
});

export function isArchivableSession(session) {
	return session.key !== "main" && (session.activeChannel !== true || session.archived === true);
}

function isClearableSession(session) {
	var isChannelSessionKey =
		session.key.startsWith("telegram:") ||
		session.key.startsWith("msteams:") ||
		session.key.startsWith("discord:") ||
		session.key.startsWith("slack:") ||
		session.key.startsWith("matrix:");
	return session.key !== "main" && !session.key.startsWith("cron:") && !isChannelSessionKey && !session.channelBinding;
}

export function clearAllSessions() {
	var allSessions = sessionStore.sessions.value;
	var count = allSessions.filter((session) => isClearableSession(session)).length;
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
			var active = sessionStore.getByKey(sessionStore.activeSessionKey.value);
			if (active && isClearableSession(active)) {
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
function restoreMcpToggle(mcpEnabled) {
	var mcpBtn = S.$("mcpToggleBtn");
	var mcpLabel = S.$("mcpToggleLabel");
	if (mcpBtn) {
		mcpBtn.style.color = mcpEnabled ? "var(--ok)" : "var(--muted)";
		mcpBtn.style.borderColor = mcpEnabled ? "var(--ok)" : "var(--border)";
	}
	if (mcpLabel) mcpLabel.textContent = mcpEnabled ? "MCP" : "MCP off";
}

// ── Switch session ──────────────────────────────────────────

function restoreSessionState(entry, projectId) {
	var effectiveProjectId = entry.projectId || projectId || "";
	projectStore.setActiveProjectId(effectiveProjectId);
	// Dual-write to state.js for backward compat
	S.setActiveProjectId(effectiveProjectId);
	localStorage.setItem("moltis-project", effectiveProjectId);
	updateSessionProjectSelect(effectiveProjectId);
	if (entry.model) {
		modelStore.select(entry.model);
		// Dual-write to state.js for backward compat
		S.setSelectedModelId(entry.model);
		localStorage.setItem("moltis-model", entry.model);
		var found = modelStore.getById(entry.model);
		if (S.modelComboLabel) S.modelComboLabel.textContent = found ? found.displayName || found.id : entry.model;
	}
	updateSandboxUI(entry.sandbox_enabled !== false);
	updateSandboxImageUI(entry.sandbox_image || null);
	var sandboxRuntimeAvailable = (S.sandboxInfo?.backend || "none") !== "none";
	var effectiveSandboxRoute = entry.sandbox_enabled !== false && sandboxRuntimeAvailable;
	S.setSessionExecMode(effectiveSandboxRoute ? "sandbox" : "host");
	S.setSessionExecPromptSymbol(effectiveSandboxRoute || S.hostExecIsRoot ? "#" : "$");
	updateCommandInputUI();
	restoreMcpToggle(!entry.mcpDisabled);
	restoreNodeSelection(entry.node_id || null);
	updateChatSessionHeader();
}

/** Extract text and images from a multimodal content array. */
function parseMultimodalContent(blocks) {
	var text = "";
	var images = [];
	for (var block of blocks) {
		if (block.type === "text") {
			text = block.text || "";
		} else if (block.type === "image_url" && block.image_url?.url) {
			images.push({ dataUrl: block.image_url.url, name: "image" });
		}
	}
	return { text: text, images: images };
}

function renderHistoryUserMessage(msg) {
	var text = "";
	var images = [];
	if (Array.isArray(msg.content)) {
		var parsed = parseMultimodalContent(msg.content);
		text = msg.channel ? stripChannelPrefix(parsed.text) : parsed.text;
		images = parsed.images;
	} else {
		text = msg.channel ? stripChannelPrefix(msg.content || "") : msg.content || "";
	}

	var el;
	if (msg.audio) {
		el = chatAddMsg("user", "", true);
		if (el) {
			var filename = msg.audio.split("/").pop();
			var audioSrc = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			renderAudioPlayer(el, audioSrc);
			if (text) {
				var textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				// Safe: renderMarkdown escapes user input before formatting tags.
				textWrap.innerHTML = renderMarkdown(text); // eslint-disable-line no-unsanitized/property
				el.appendChild(textWrap);
			}
			if (images.length > 0) {
				var thumbRow = document.createElement("div");
				thumbRow.className = "msg-image-row";
				for (var img of images) {
					var thumb = document.createElement("img");
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

function createModelFooter(msg) {
	var ft = document.createElement("div");
	ft.className = "msg-model-footer";
	var ftText = msg.provider ? `${msg.provider} / ${msg.model}` : msg.model;
	if (msg.inputTokens || msg.outputTokens) {
		ftText += ` \u00b7 ${formatAssistantTokenUsage(msg.inputTokens, msg.outputTokens, msg.cacheReadTokens)}`;
	}
	var textSpan = document.createElement("span");
	textSpan.textContent = ftText;
	ft.appendChild(textSpan);

	var speedLabel = formatTokenSpeed(msg.outputTokens || 0, msg.durationMs || 0);
	if (speedLabel) {
		var speed = document.createElement("span");
		speed.className = "msg-token-speed";
		var tone = tokenSpeedTone(msg.outputTokens || 0, msg.durationMs || 0);
		if (tone) speed.classList.add(`msg-token-speed-${tone}`);
		speed.textContent = ` \u00b7 ${speedLabel}`;
		ft.appendChild(speed);
	}
	return ft;
}

function renderHistoryAssistantMessage(msg) {
	var el;
	if (msg.audio) {
		// Voice response: render audio player first, then transcript text below.
		el = chatAddMsg("assistant", "", true);
		if (el) {
			var filename = msg.audio.split("/").pop();
			var audioSrc = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			renderAudioPlayer(el, audioSrc);
			if (msg.content) {
				var textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				// Safe: renderMarkdown calls esc() first — all user input is HTML-escaped.
				textWrap.innerHTML = renderMarkdown(msg.content); // eslint-disable-line no-unsanitized/property
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
		var footer = createModelFooter(msg);
		el.appendChild(footer);
		void attachMessageVoiceControl({
			messageEl: el,
			footerEl: footer,
			sessionKey: S.activeSessionKey,
			text: msg.content || "",
			runId: msg.run_id || null,
			messageIndex: msg.historyIndex,
			audioPath: msg.audio || null,
			audioWarning: null,
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
function renderHistoryToolResult(msg) {
	var tpl = document.getElementById("tpl-exec-card");
	var frag = tpl.content.cloneNode(true);
	var card = frag.firstElementChild;

	// Remove the "running…" status element — this is a completed result.
	var statusEl = card.querySelector(".exec-status");
	if (statusEl) statusEl.remove();

	// Set command summary from arguments.
	var cmd = toolCallSummary(msg.tool_name, msg.arguments);
	card.querySelector("[data-cmd]").textContent = ` ${cmd}`;

	// Set success/error CSS class (replace the default "running" class).
	card.className = `msg exec-card ${msg.success ? "exec-ok" : "exec-err"}`;

	// Append result output if present.
	if (msg.result) {
		var out = (msg.result.stdout || "").replace(/\n+$/, "");
		if (out) {
			var outEl = document.createElement("pre");
			outEl.className = "exec-output";
			outEl.textContent = out;
			card.appendChild(outEl);
		}
		var stderrText = (msg.result.stderr || "").replace(/\n+$/, "");
		if (stderrText) {
			var errEl = document.createElement("pre");
			errEl.className = "exec-output exec-stderr";
			errEl.textContent = stderrText;
			card.appendChild(errEl);
		}
		if (msg.result.exit_code !== undefined && msg.result.exit_code !== 0) {
			var codeEl = document.createElement("div");
			codeEl.className = "exec-exit";
			codeEl.textContent = `exit ${msg.result.exit_code}`;
			card.appendChild(codeEl);
		}
		// Render persisted screenshot from the media API.
		if (msg.result.screenshot && !msg.result.screenshot.startsWith("data:")) {
			var filename = msg.result.screenshot.split("/").pop();
			var sessionKey = S.activeSessionKey || "main";
			var mediaSrc = `/api/sessions/${encodeURIComponent(sessionKey)}/media/${encodeURIComponent(filename)}`;
			renderScreenshot(card, mediaSrc);
		}
		// Render persisted document from the media API.
		if (msg.result.document_ref) {
			var docStoredName = msg.result.document_ref.split("/").pop();
			var docDisplayName = msg.result.filename || docStoredName;
			var docSessionKey = S.activeSessionKey || "main";
			var docMediaSrc = `/api/sessions/${encodeURIComponent(docSessionKey)}/media/${encodeURIComponent(docStoredName)}`;
			renderDocument(card, docMediaSrc, docDisplayName, msg.result.mime_type, msg.result.size_bytes);
		}
	}

	// Append error detail if present.
	if (!msg.success && msg.error) {
		var errMsg = document.createElement("div");
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

export function appendLastMessageTimestamp(epochMs) {
	if (!S.chatMsgBox) return;
	// Remove any previous last-message timestamp
	var old = S.chatMsgBox.querySelector(".msg-footer-time");
	if (old) old.remove();
	var lastMsg = S.chatMsgBox.lastElementChild;
	if (!lastMsg || lastMsg.classList.contains("user")) return;
	var footer = lastMsg.querySelector(".msg-model-footer");
	if (!footer) {
		footer = document.createElement("div");
		footer.className = "msg-model-footer";
		lastMsg.appendChild(footer);
	}
	var sep = document.createTextNode(" \u00b7 ");
	sep.className = "msg-footer-time";
	var t = document.createElement("time");
	t.className = "msg-footer-time";
	t.setAttribute("data-epoch-ms", String(epochMs));
	t.textContent = new Date(epochMs).toISOString();
	// Wrap separator + time in a span so we can remove both easily
	var wrap = document.createElement("span");
	wrap.className = "msg-footer-time";
	wrap.appendChild(document.createTextNode(" \u00b7 "));
	wrap.appendChild(t);
	footer.appendChild(wrap);
}

function makeThinkingDots() {
	var tpl = document.getElementById("tpl-thinking-dots");
	return tpl.content.cloneNode(true).firstElementChild;
}

function postHistoryLoadActions(key, searchContext, msgEls, thinkingText, skipAutoScroll) {
	sendRpc("chat.context", {}).then((ctxRes) => {
		if (ctxRes?.ok && ctxRes.payload) {
			if (ctxRes.payload.tokenUsage) {
				var tu = ctxRes.payload.tokenUsage;
				S.setSessionContextWindow(tu.contextWindow || 0);
				S.setSessionTokens({
					input: tu.inputTokens || 0,
					output: tu.outputTokens || 0,
				});
				S.setSessionCurrentInputTokens(tu.estimatedNextInputTokens || tu.currentInputTokens || tu.inputTokens || 0);
			}
			S.setSessionToolsEnabled(ctxRes.payload.supportsTools !== false);
			var execution = ctxRes.payload.execution || {};
			var mode = execution.mode === "sandbox" ? "sandbox" : "host";
			var hostIsRoot = execution.hostIsRoot === true;
			var isRoot = execution.isRoot;
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

	var session = sessionStore.getByKey(key);
	if (session?.replying.value && S.chatMsgBox) {
		removeThinking();
		var thinkEl = document.createElement("div");
		thinkEl.className = "msg assistant thinking";
		thinkEl.id = "thinkingIndicator";
		if (thinkingText) {
			var textEl = document.createElement("span");
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

function mergeHistoryPages(existingHistory, olderHistory) {
	var older = Array.isArray(olderHistory) ? olderHistory : [];
	var current = Array.isArray(existingHistory) ? existingHistory : [];
	if (older.length === 0) return current;
	if (current.length === 0) return older;

	var byIndex = new Map();
	var ordered = [];
	var pushMessage = (msg) => {
		var idx = historyIndexFromMessage(msg);
		if (idx === null) {
			ordered.push(msg);
			return;
		}
		if (!byIndex.has(idx)) {
			ordered.push(msg);
		}
		byIndex.set(idx, msg);
	};

	for (var olderMsg of older) pushMessage(olderMsg);
	for (var currentMsg of current) pushMessage(currentMsg);

	return ordered.map((msg) => {
		var idx = historyIndexFromMessage(msg);
		if (idx === null) return msg;
		return byIndex.get(idx) || msg;
	});
}

function canLoadOlderHistory(key) {
	var paging = getHistoryPaginationState(key);
	if (!(paging && paging.hasMore && Number.isInteger(paging.nextCursor))) return false;
	if (paging.loadingOlder) return false;
	return true;
}

function maybeLoadOlderHistoryFromScroll() {
	if (!S.chatMsgBox) return;
	if (S.chatMsgBox.scrollTop > HISTORY_AUTOLOAD_THRESHOLD_PX) return;
	var key = sessionStore.activeSessionKey.value || S.activeSessionKey;
	if (!key) return;
	if (!canLoadOlderHistory(key)) return;
	void loadOlderHistoryPage(key);
}

function handleHistoryScroll() {
	if (historyScrollRaf) return;
	historyScrollRaf = requestAnimationFrame(() => {
		historyScrollRaf = 0;
		maybeLoadOlderHistoryFromScroll();
	});
}

function ensureHistoryScrollBinding() {
	var nextEl = S.chatMsgBox;
	if (historyScrollEl === nextEl) return;
	if (historyScrollEl) {
		historyScrollEl.removeEventListener("scroll", handleHistoryScroll);
	}
	historyScrollEl = nextEl;
	if (!historyScrollEl) return;
	historyScrollEl.addEventListener("scroll", handleHistoryScroll, { passive: true });
}

async function loadOlderHistoryPage(key) {
	if (!canLoadOlderHistory(key)) return;
	var paging = getHistoryPaginationState(key);
	if (!paging) return;
	if (sessionStore.activeSessionKey.value !== key) return;

	var nextState = { ...paging, loadingOlder: true };
	sessionHistoryPaging.set(key, nextState);
	var loadedHistory = getSessionHistory(key) || [];
	var totalBefore = Number.isInteger(nextState.totalMessages) ? nextState.totalMessages : loadedHistory.length;
	renderHistory(key, loadedHistory, null, null, totalBefore, true);

	var beforeHeight = S.chatMsgBox ? S.chatMsgBox.scrollHeight : 0;
	var beforeTop = S.chatMsgBox ? S.chatMsgBox.scrollTop : 0;

	try {
		var payload = await fetchSessionHistoryViaHttp(key, {
			cursor: nextState.nextCursor,
			limit: SESSION_HISTORY_PAGE_LIMIT,
		});
		if (sessionStore.activeSessionKey.value !== key) return;

		var older = Array.isArray(payload.history) ? payload.history : [];
		var current = getSessionHistory(key) || [];
		if (older.length > 0 && payload.historyCacheHit !== true) {
			replaceSessionHistory(key, mergeHistoryPages(current, older));
		}
		setHistoryPaginationState(key, payload);

		var merged = getSessionHistory(key) || [];
		var sessionEntry = sessionStore.getByKey(key);
		var totalCountHint = Number.isInteger(sessionEntry?.messageCount)
			? sessionEntry.messageCount
			: Number(payload.totalMessages) || merged.length;
		renderHistory(key, merged, null, null, totalCountHint, true);

		if (S.chatMsgBox) {
			var afterHeight = S.chatMsgBox.scrollHeight;
			S.chatMsgBox.scrollTop = Math.max(0, beforeTop + (afterHeight - beforeHeight));
		}
	} catch {
		if (sessionStore.activeSessionKey.value !== key) return;
		var fallback = getSessionHistory(key) || [];
		var fallbackTotal = Number.isInteger(nextState.totalMessages) ? nextState.totalMessages : fallback.length;
		sessionHistoryPaging.set(key, { ...nextState, loadingOlder: false });
		renderHistory(key, fallback, null, null, fallbackTotal, true);
		chatAddMsg("error", "Failed to load older messages");
	} finally {
		var latest = getHistoryPaginationState(key);
		if (latest) sessionHistoryPaging.set(key, { ...latest, loadingOlder: false });
		if (sessionStore.activeSessionKey.value === key) {
			maybeLoadOlderHistoryFromScroll();
		}
	}
}

/** No-op — the Preact SessionHeader component auto-updates from signals. */
export function updateChatSessionHeader() {
	// Retained for backward compat call sites; Preact handles rendering.
}

function renderWelcomeAgentPicker(card, activeAgentId, onActiveAgentResolved) {
	var container = card.querySelector("[data-welcome-agents]");
	if (!container) return;

	sendRpc("agents.list", {}).then((res) => {
		if (!card.isConnected) return;
		if (!res?.ok) {
			container.classList.add("hidden");
			return;
		}
		var parsed = parseAgentsListPayload(res.payload);
		var agents = parsed.agents || [];
		var defaultId = parsed.defaultId || "main";
		var effectiveActive = activeAgentId || defaultId;

		container.textContent = "";
		container.classList.remove("hidden");
		container.classList.add("flex");

		var activeAgent = null;
		for (const agent of agents) {
			if (!agent?.id) continue;
			if (agent.id === effectiveActive) activeAgent = agent;
			var chip = document.createElement("button");
			chip.type = "button";
			chip.className = agent.id === effectiveActive ? "provider-btn" : "provider-btn provider-btn-secondary";
			chip.style.fontSize = "0.7rem";
			chip.style.padding = "3px 8px";
			var labelPrefix = agent.emoji ? `${agent.emoji} ` : "";
			chip.textContent = `${labelPrefix}${agent.name || agent.id}`;
			chip.addEventListener("click", () => {
				var key = sessionStore.activeSessionKey.value || S.activeSessionKey || "main";
				sendRpc("agents.set_session", { session_key: key, agent_id: agent.id }).then((setRes) => {
					if (!setRes?.ok) return;
					var live = sessionStore.getByKey(key);
					if (live) {
						live.agent_id = agent.id;
						live.dataVersion.value++;
					}
					fetchSessions();
					var welcome = S.chatMsgBox?.querySelector("#welcomeCard");
					if (welcome) {
						welcome.remove();
						showWelcomeCard();
					}
				});
			});
			container.appendChild(chip);
		}

		var hatchBtn = document.createElement("button");
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

function showWelcomeCard() {
	if (!S.chatMsgBox) return;
	S.chatMsgBox.classList.add("chat-messages-empty");

	if (modelStore.models.value.length === 0) {
		var noProvTpl = document.getElementById("tpl-no-providers-card");
		if (!noProvTpl) return;
		var noProvCard = noProvTpl.content.cloneNode(true).firstElementChild;
		S.chatMsgBox.appendChild(noProvCard);
		return;
	}

	var tpl = document.getElementById("tpl-welcome-card");
	if (!tpl) return;
	var card = tpl.content.cloneNode(true).firstElementChild;
	var identity = gon.get("identity");
	var userName = identity?.user_name;
	var botName = identity?.name || "moltis";
	var botEmoji = identity?.emoji || "";

	var greetingEl = card.querySelector("[data-welcome-greeting]");
	if (greetingEl) greetingEl.textContent = userName ? `Hello, ${userName}!` : "Hello!";
	var emojiEl = card.querySelector("[data-welcome-emoji]");
	if (emojiEl) emojiEl.textContent = botEmoji;
	var nameEl = card.querySelector("[data-welcome-bot-name]");
	if (nameEl) nameEl.textContent = botName;
	var activeAgentId = sessionStore.activeSession.value?.agent_id || "main";
	renderWelcomeAgentPicker(card, activeAgentId, (activeAgent) => {
		if (!activeAgent) return;
		if (emojiEl) emojiEl.textContent = activeAgent.emoji || "";
		if (nameEl) nameEl.textContent = activeAgent.name || botName;
	});

	S.chatMsgBox.appendChild(card);
}

export function refreshWelcomeCardIfNeeded() {
	if (!S.chatMsgBox) return;
	var welcomeCard = S.chatMsgBox.querySelector("#welcomeCard");
	var noProvCard = S.chatMsgBox.querySelector("#noProvidersCard");
	var hasModels = modelStore.models.value.length > 0;

	// Wrong variant showing — swap it
	if (hasModels && noProvCard) {
		noProvCard.remove();
		showWelcomeCard();
	} else if (!hasModels && welcomeCard) {
		welcomeCard.remove();
		showWelcomeCard();
	}
}

function ensureSessionInClientStore(key, entry, projectId) {
	var existing = sessionStore.getByKey(key);
	if (existing) return existing;

	var created = { ...entry, key: key };
	if (projectId && !created.projectId) created.projectId = projectId;
	var createdSession = sessionStore.upsert(created);

	// Keep state.js mirror in sync for legacy call sites.
	var inLegacy = S.sessions.some((s) => s.key === key);
	if (!inLegacy) {
		S.setSessions(insertSessionInOrder(S.sessions, created));
	}
	return createdSession;
}

function showSessionLoadIndicator() {
	if (!S.chatMsgBox) return;
	hideSessionLoadIndicator();
	var loading = document.createElement("div");
	loading.id = "sessionLoadIndicator";
	loading.className = "msg assistant thinking session-loading";
	loading.appendChild(makeThinkingDots());
	var label = document.createElement("span");
	label.className = "session-loading-label";
	label.textContent = "Loading session…";
	loading.appendChild(label);
	S.chatMsgBox.appendChild(loading);
}

function hideSessionLoadIndicator() {
	var loading = document.getElementById("sessionLoadIndicator");
	if (loading) loading.remove();
}

function startSwitchRequest(key) {
	switchRequestSeq += 1;
	latestSwitchRequestBySession.set(key, switchRequestSeq);
	return switchRequestSeq;
}

function isLatestSwitchRequest(key, requestId) {
	return latestSwitchRequestBySession.get(key) === requestId;
}

function startSessionRefresh(key, blockRealtimeEvents) {
	sessionStore.refreshInProgressKey.value = key;
	sessionStore.switchInProgress.value = !!blockRealtimeEvents;
	S.setSessionSwitchInProgress(!!blockRealtimeEvents);
}

function finishSessionRefresh(key) {
	if (sessionStore.refreshInProgressKey.value !== key) return;
	sessionStore.refreshInProgressKey.value = "";
	sessionStore.switchInProgress.value = false;
	S.setSessionSwitchInProgress(false);
}

function resetSwitchViewState() {
	hideSessionLoadIndicator();
	if (S.chatMsgBox) S.chatMsgBox.textContent = "";
	var tray = document.getElementById("queuedMessages");
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

function syncHistoryState(key, history, historyTailIndex, totalCountHint) {
	var loadedCount = Array.isArray(history) ? history.length : 0;
	var sessionEntry = sessionStore.getByKey(key);
	var legacy = S.sessions.find((s) => s.key === key);
	var existingCount = Number.isInteger(sessionEntry?.messageCount) ? sessionEntry.messageCount : 0;
	var legacyCount = Number.isInteger(legacy?.messageCount) ? legacy.messageCount : 0;
	var hintedCount = Number.isInteger(totalCountHint) ? totalCountHint : 0;
	var count = Math.max(loadedCount, existingCount, hintedCount, legacyCount);
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

function renderHistory(key, history, searchContext, thinkingText, totalCountHint, skipAutoScroll) {
	ensureHistoryScrollBinding();
	hideSessionLoadIndicator();
	if (S.chatMsgBox) {
		S.chatMsgBox.classList.remove("chat-messages-empty");
		S.chatMsgBox.textContent = "";
	}
	var msgEls = [];
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionCurrentInputTokens(0);
	S.setChatBatchLoading(true);
	history.forEach((msg) => {
		if (msg.role === "user") {
			msgEls.push(renderHistoryUserMessage(msg));
		} else if (msg.role === "assistant") {
			msgEls.push(renderHistoryAssistantMessage(msg));
		} else if (msg.role === "notice") {
			msgEls.push(chatAddMsg("system", renderMarkdown(msg.content || ""), true));
		} else if (msg.role === "tool_result") {
			msgEls.push(renderHistoryToolResult(msg));
		} else {
			msgEls.push(null);
		}
	});
	S.setChatBatchLoading(false);
	// Syntax-highlight all code blocks in the rendered history.
	if (S.chatMsgBox) highlightCodeBlocks(S.chatMsgBox);
	var historyTailIndex = computeHistoryTailIndex(history);
	syncHistoryState(key, history, historyTailIndex, totalCountHint);

	// Resume chatSeq from the highest user message seq in history
	// so the counter continues from where it left off after reload.
	var maxSeq = 0;
	for (var hm of history) {
		if (hm.role === "user" && hm.seq > maxSeq) {
			maxSeq = hm.seq;
		}
	}
	S.setChatSeq(maxSeq);
	if (history.length === 0) {
		showWelcomeCard();
	} else {
		var lastMsg = history[history.length - 1];
		var ts = lastMsg.created_at;
		if (ts) appendLastMessageTimestamp(ts);
	}
	postHistoryLoadActions(key, searchContext, msgEls, thinkingText, skipAutoScroll === true);
}

function shouldApplyServerHistory(key, serverHistory, requestRevision) {
	var current = getSessionHistory(key);
	if (!current) return true;
	var serverTail = computeHistoryTailIndex(serverHistory);
	var currentTail = computeHistoryTailIndex(current);
	if (serverTail > currentTail) return true;
	if (serverTail < currentTail) return false;
	var currentRevision = getHistoryRevision(key);
	if (currentRevision === requestRevision) return true;
	return !historyHasUnindexedMessages(current);
}

function applyReplyingStateFromSwitchPayload(key, payload) {
	var replying = payload.replying === true;
	setSessionReplying(key, replying);
	var voiceSession = sessionStore.getByKey(key);
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

async function fetchSessionHistoryViaHttp(key, options) {
	var opts = options || {};
	var query = new URLSearchParams();
	if (Number.isInteger(opts.cachedMessageCount) && opts.cachedMessageCount >= 0) {
		query.set("cached_message_count", String(opts.cachedMessageCount));
	}
	if (Number.isInteger(opts.cursor) && opts.cursor >= 0) {
		query.set("cursor", String(opts.cursor));
	}
	if (Number.isInteger(opts.limit) && opts.limit > 0) {
		query.set("limit", String(opts.limit));
	}
	var url = `/api/sessions/${encodeURIComponent(key)}/history`;
	var qs = query.toString();
	if (qs) url += `?${qs}`;

	var response = await fetch(url, {
		headers: { Accept: "application/json" },
	});
	var payload = null;
	try {
		payload = await response.json();
	} catch {
		payload = null;
	}
	if (!response.ok) {
		var errMsg = payload?.error || `Failed to load session history (${response.status})`;
		throw new Error(errMsg);
	}
	return payload || {};
}

export function switchSession(key, searchContext, projectId) {
	sessionStore.setActive(key);
	// Dual-write to state.js for backward compat
	S.setActiveSessionKey(key);
	localStorage.setItem("moltis-session", key);
	history.replaceState(null, "", sessionPath(key));
	resetSwitchViewState();
	var cachedEntry = sessionStore.getByKey(key);
	if (cachedEntry) {
		restoreSessionState(cachedEntry, projectId);
	}
	// Preact SessionList auto-rerenders active/unread from signals.

	var switchReqId = startSwitchRequest(key);
	var switchParams = { key: key };
	if (projectId) switchParams.project_id = projectId;
	// Keep WebSocket for live state only; history comes from HTTP (gzip-friendly).
	switchParams.include_history = false;
	var cachedHistory = getSessionHistory(key);
	var hasCache = Array.isArray(cachedHistory);
	var cacheRevisionAtRequest = getHistoryRevision(key);
	var cacheComplete = hasCache && isHistoryCacheComplete(key);
	var cachedHistoryCount = cacheComplete
		? Number.isInteger(cachedEntry?.messageCount)
			? cachedEntry.messageCount
			: cachedHistory.length
		: null;
	startSessionRefresh(key, !hasCache);
	if (hasCache) {
		renderHistory(key, cachedHistory, searchContext, null, cachedHistoryCount, false);
	} else {
		showSessionLoadIndicator();
	}

	sendRpc("sessions.switch", switchParams)
		.then(async (res) => {
			if (!isLatestSwitchRequest(key, switchReqId)) return;
			var stillActive = sessionStore.activeSessionKey.value === key;
			if (!(res?.ok && res.payload)) {
				if (stillActive && !hasCache) {
					hideSessionLoadIndicator();
					chatAddMsg("error", res?.error?.message || "Failed to load session");
				}
				finishSessionRefresh(key);
				if (stillActive && S.chatInput) S.chatInput.focus();
				return;
			}

			var entry = res.payload.entry || {};
			ensureSessionInClientStore(key, entry, projectId);
			var pagingBefore = getHistoryPaginationState(key);
			var pagingBeforeHasMore = pagingBefore?.hasMore === true;
			var pagingBeforeCursor = Number.isInteger(pagingBefore?.nextCursor) ? pagingBefore.nextCursor : null;
			var historyPayload = {
				historyCacheHit: res.payload.historyCacheHit === true,
				history: Array.isArray(res.payload.history) ? res.payload.history : [],
				historyTruncated: res.payload.historyTruncated === true,
				historyDroppedCount: Number(res.payload.historyDroppedCount) || 0,
			};
			if (res.payload.historyOmitted === true) {
				try {
					historyPayload = await fetchSessionHistoryViaHttp(key, {
						cachedMessageCount: cachedHistoryCount,
						limit: SESSION_HISTORY_PAGE_LIMIT,
					});
				} catch (error) {
					if (!isLatestSwitchRequest(key, switchReqId)) return;
					stillActive = sessionStore.activeSessionKey.value === key;
					if (stillActive && !hasCache) {
						hideSessionLoadIndicator();
						chatAddMsg("error", error?.message || "Failed to load session history");
					}
					finishSessionRefresh(key);
					if (stillActive && S.chatInput) S.chatInput.focus();
					return;
				}
				if (!isLatestSwitchRequest(key, switchReqId)) return;
				stillActive = sessionStore.activeSessionKey.value === key;
			}
			setHistoryPaginationState(key, historyPayload);
			var pagingAfter = getHistoryPaginationState(key);
			var pagingAfterHasMore = pagingAfter?.hasMore === true;
			var pagingAfterCursor = Number.isInteger(pagingAfter?.nextCursor) ? pagingAfter.nextCursor : null;
			var paginationChanged = pagingBeforeHasMore !== pagingAfterHasMore || pagingBeforeCursor !== pagingAfterCursor;

			var cacheHit = historyPayload.historyCacheHit === true;
			var serverHistory = Array.isArray(historyPayload.history) ? historyPayload.history : [];
			var appliedServerHistory = false;
			if (!cacheHit && shouldApplyServerHistory(key, serverHistory, cacheRevisionAtRequest)) {
				replaceSessionHistory(key, serverHistory);
				appliedServerHistory = true;
			}
			var history = getSessionHistory(key) || serverHistory;
			if (stillActive) {
				restoreSessionState(entry, projectId);
				applyReplyingStateFromSwitchPayload(key, res.payload);
				var thinkingText = res.payload.replying ? res.payload.thinkingText || null : null;
				var totalCountHint = Number.isInteger(entry.messageCount)
					? entry.messageCount
					: Number(historyPayload.totalMessages) || history.length;
				var shouldRerender = !hasCache || Boolean(searchContext?.query) || appliedServerHistory || paginationChanged;
				if (shouldRerender) {
					renderHistory(key, history, searchContext, thinkingText, totalCountHint, false);
				} else {
					postHistoryLoadActions(key, searchContext, [], thinkingText, false);
				}
				if (appliedServerHistory && historyPayload.historyTruncated === true) {
					var dropped = Number(historyPayload.historyDroppedCount) || 0;
					chatAddMsg(
						"system",
						`Loaded the most recent messages for performance (${dropped} older message${dropped === 1 ? "" : "s"} omitted).`,
					);
				}
				if (appliedServerHistory && historyPayload.hasMore === true) {
					var total = Number(historyPayload.totalMessages) || history.length;
					chatAddMsg("system", `Loaded recent history (${history.length} of ${total} messages) for faster loading.`);
				}
				if (S.chatInput) S.chatInput.focus();
			}
			finishSessionRefresh(key);
		})
		.catch(() => {
			if (!isLatestSwitchRequest(key, switchReqId)) return;
			var stillActive = sessionStore.activeSessionKey.value === key;
			if (stillActive && !hasCache) {
				hideSessionLoadIndicator();
				chatAddMsg("error", "Failed to load session");
			}
			finishSessionRefresh(key);
			if (stillActive && S.chatInput) S.chatInput.focus();
		});
}
