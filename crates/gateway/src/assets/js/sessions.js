// ── Sessions: list, switch, status helpers ──────────────────

import {
	appendChannelFooter,
	chatAddMsg,
	chatAddMsgWithImages,
	highlightAndScroll,
	removeThinking,
	scrollChatToBottom,
	stripChannelPrefix,
	updateTokenBar,
} from "./chat-ui.js";
import * as gon from "./gon.js";
import {
	formatTokens,
	renderAudioPlayer,
	renderMarkdown,
	renderScreenshot,
	sendRpc,
	toolCallSummary,
} from "./helpers.js";
import { updateSessionProjectSelect } from "./project-combo.js";
import { currentPrefix, navigate, sessionPath } from "./router.js";
import { updateSandboxImageUI, updateSandboxUI } from "./sandbox.js";
import * as S from "./state.js";
import { modelStore } from "./stores/model-store.js";
import { projectStore } from "./stores/project-store.js";
import { sessionStore } from "./stores/session-store.js";
import { confirmDialog } from "./ui.js";

var SESSION_PREVIEW_MAX_CHARS = 200;

function truncateSessionPreview(text) {
	var trimmed = (text || "").trim();
	if (!trimmed) return "";
	var chars = Array.from(trimmed);
	if (chars.length <= SESSION_PREVIEW_MAX_CHARS) return trimmed;
	return `${chars.slice(0, SESSION_PREVIEW_MAX_CHARS).join("")}…`;
}

// ── Fetch & render ──────────────────────────────────────────

export function fetchSessions() {
	sendRpc("sessions.list", {}).then((res) => {
		if (!res?.ok) return;
		var incoming = res.payload || [];
		// Preserve client-side flags (localUnread, replying) across fetches.
		var oldByKey = {};
		for (var old of S.sessions) {
			if (old._localUnread || old._replying) {
				oldByKey[old.key] = {
					localUnread: old._localUnread,
					replying: old._replying,
				};
			}
		}
		for (var s of incoming) {
			var prev = oldByKey[s.key];
			if (prev) {
				if (prev.localUnread) s._localUnread = true;
				if (prev.replying) s._replying = true;
			}
		}
		// Update session store (source of truth) — version guard
		// inside Session.update() prevents stale data from overwriting.
		sessionStore.setAll(incoming);
		// Dual-write to state.js for backward compat
		S.setSessions(incoming);
		renderSessionList();
		updateChatSessionHeader();
	});
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
			updateTokenBar();
			var activeKey = sessionStore.activeSessionKey.value || S.activeSessionKey;
			var session = sessionStore.getByKey(activeKey);
			if (session) session.syncCounts(0, 0);
			fetchSessions();
			return res;
		}
		S.setLastHistoryIndex(prevHistoryIdx);
		S.setChatSeq(prevSeq);
		chatAddMsg("error", res?.error?.message || "Clear failed");
		return res;
	});
}

/** Re-fetch the active session entry and restore sandbox/model state. */
export function refreshActiveSession() {
	if (!S.activeSessionKey) return;
	sendRpc("sessions.resolve", { key: S.activeSessionKey }).then((res) => {
		if (!(res?.ok && res.payload)) return;
		var entry = res.payload.entry || res.payload;
		restoreSessionState(entry, entry.projectId);
	});
}

// ── Session list ─────────────────────────────────────────────
// The Preact SessionList component is mounted once from app.js and
// auto-rerenders from signals.  This function handles the imperative
// Clear button visibility that lives outside the component.

export function renderSessionList() {
	updateClearAllVisibility();
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

// ── New session button ──────────────────────────────────────
var newSessionBtn = S.$("newSessionBtn");
newSessionBtn.addEventListener("click", () => {
	var key = `session:${crypto.randomUUID()}`;
	var filterId = projectStore.projectFilterId.value;
	if (currentPrefix === "/chats") {
		switchSession(key, null, filterId || undefined);
	} else {
		navigate(sessionPath(key));
	}
});

// ── Clear all sessions button ───────────────────────────────
var clearAllBtn = S.$("clearAllSessionsBtn");

/** Show the Clear button only when there are deletable (session:*) sessions. */
function updateClearAllVisibility() {
	if (!clearAllBtn) return;
	var allSessions = sessionStore.sessions.value;
	var hasClearable = allSessions.some(
		(s) => s.key !== "main" && !s.key.startsWith("cron:") && !s.key.startsWith("telegram:") && !s.channelBinding,
	);
	clearAllBtn.classList.toggle("hidden", !hasClearable);
}

if (clearAllBtn) {
	clearAllBtn.addEventListener("click", () => {
		var allSessions = sessionStore.sessions.value;
		var count = allSessions.filter(
			(s) => s.key !== "main" && !s.key.startsWith("cron:") && !s.key.startsWith("telegram:") && !s.channelBinding,
		).length;
		if (count === 0) return;
		confirmDialog(
			`Delete ${count} session${count !== 1 ? "s" : ""}? Main, Telegram and cron sessions will be kept.`,
		).then((yes) => {
			if (!yes) return;
			clearAllBtn.disabled = true;
			clearAllBtn.textContent = "Clearing\u2026";
			sendRpc("sessions.clear_all", {}).then((res) => {
				clearAllBtn.disabled = false;
				clearAllBtn.textContent = "Clear";
				if (res?.ok) {
					// If the active session was deleted, switch to main.
					var active = sessionStore.getByKey(sessionStore.activeSessionKey.value);
					var wasKept =
						!active ||
						active.key === "main" ||
						active.key.startsWith("cron:") ||
						active.key.startsWith("telegram:") ||
						active.channelBinding;
					if (!wasKept) {
						switchSession("main");
					}
					fetchSessions();
				}
			});
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
	restoreMcpToggle(!entry.mcpDisabled);
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
	var el;
	if (Array.isArray(msg.content)) {
		var parsed = parseMultimodalContent(msg.content);
		var text = msg.channel ? stripChannelPrefix(parsed.text) : parsed.text;
		el = chatAddMsgWithImages("user", text ? renderMarkdown(text) : "", parsed.images);
	} else {
		var userContent = msg.channel ? stripChannelPrefix(msg.content || "") : msg.content || "";
		el = chatAddMsg("user", renderMarkdown(userContent), true);
	}
	if (el && msg.channel) appendChannelFooter(el, msg.channel);
	return el;
}

function createModelFooter(msg) {
	var ft = document.createElement("div");
	ft.className = "msg-model-footer";
	var ftText = msg.provider ? `${msg.provider} / ${msg.model}` : msg.model;
	if (msg.inputTokens || msg.outputTokens) {
		ftText += ` \u00b7 ${formatTokens(msg.inputTokens || 0)} in / ${formatTokens(msg.outputTokens || 0)} out`;
	}
	ft.textContent = ftText;
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
		}
	} else {
		el = chatAddMsg("assistant", renderMarkdown(msg.content || ""), true);
	}
	if (el && msg.model) {
		el.appendChild(createModelFooter(msg));
	}
	if (msg.inputTokens || msg.outputTokens) {
		S.sessionTokens.input += msg.inputTokens || 0;
		S.sessionTokens.output += msg.outputTokens || 0;
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
	}

	// Append error detail if present.
	if (!msg.success && msg.error) {
		var errMsg = document.createElement("div");
		errMsg.className = "exec-error-detail";
		errMsg.textContent = msg.error;
		card.appendChild(errMsg);
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

function postHistoryLoadActions(key, searchContext, msgEls) {
	sendRpc("chat.context", {}).then((ctxRes) => {
		if (ctxRes?.ok && ctxRes.payload) {
			if (ctxRes.payload.tokenUsage) {
				S.setSessionContextWindow(ctxRes.payload.tokenUsage.contextWindow || 0);
			}
			S.setSessionToolsEnabled(ctxRes.payload.supportsTools !== false);
		}
		updateTokenBar();
	});
	updateTokenBar();

	if (searchContext?.query && S.chatMsgBox) {
		highlightAndScroll(msgEls, searchContext.messageIndex, searchContext.query);
	} else {
		scrollChatToBottom();
	}

	var session = sessionStore.getByKey(key);
	if (session?.replying.value && S.chatMsgBox) {
		removeThinking();
		var thinkEl = document.createElement("div");
		thinkEl.className = "msg assistant thinking";
		thinkEl.id = "thinkingIndicator";
		thinkEl.appendChild(makeThinkingDots());
		S.chatMsgBox.appendChild(thinkEl);
		scrollChatToBottom();
	}
}

/** No-op — the Preact SessionHeader component auto-updates from signals. */
export function updateChatSessionHeader() {
	// Retained for backward compat call sites; Preact handles rendering.
}

function showWelcomeCard() {
	if (!S.chatMsgBox) return;

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
		S.setSessions([...S.sessions, created]);
	}
	return createdSession;
}

export function switchSession(key, searchContext, projectId) {
	sessionStore.setActive(key);
	// Dual-write to state.js for backward compat
	S.setActiveSessionKey(key);
	localStorage.setItem("moltis-session", key);
	history.replaceState(null, "", sessionPath(key));
	if (S.chatMsgBox) S.chatMsgBox.textContent = "";
	var tray = document.getElementById("queuedMessages");
	if (tray) {
		while (tray.firstChild) tray.removeChild(tray.firstChild);
		tray.classList.add("hidden");
	}
	S.setStreamEl(null);
	S.setStreamText("");
	S.setLastHistoryIndex(-1);
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionContextWindow(0);
	updateTokenBar();
	// Preact SessionList auto-rerenders active/unread from signals.

	sessionStore.switchInProgress.value = true;
	S.setSessionSwitchInProgress(true);
	var switchParams = { key: key };
	if (projectId) switchParams.project_id = projectId;
	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Session switch handles many state updates
	sendRpc("sessions.switch", switchParams).then((res) => {
		if (res?.ok && res.payload) {
			var entry = res.payload.entry || {};
			ensureSessionInClientStore(key, entry, projectId);
			restoreSessionState(entry, projectId);
			var history = res.payload.history || [];
			var msgEls = [];
			S.setSessionTokens({ input: 0, output: 0 });
			S.setChatBatchLoading(true);
			history.forEach((msg) => {
				if (msg.role === "user") {
					msgEls.push(renderHistoryUserMessage(msg));
				} else if (msg.role === "assistant") {
					msgEls.push(renderHistoryAssistantMessage(msg));
				} else if (msg.role === "tool_result") {
					msgEls.push(renderHistoryToolResult(msg));
				} else {
					msgEls.push(null);
				}
			});
			S.setChatBatchLoading(false);
			S.setLastHistoryIndex(history.length > 0 ? history.length - 1 : -1);
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
			}
			if (history.length > 0) {
				var lastMsg = history[history.length - 1];
				var ts = lastMsg.created_at;
				if (ts) appendLastMessageTimestamp(ts);
			}
			// Sync the store entry — syncCounts calls updateBadge() for re-render.
			var sessionEntry = sessionStore.getByKey(key);
			if (sessionEntry) {
				sessionEntry.syncCounts(history.length, history.length);
				sessionEntry.localUnread.value = false;
				sessionEntry.lastHistoryIndex.value = history.length > 0 ? history.length - 1 : -1;
			}
			// Also sync the plain S.sessions entry for backward compat
			var sEntry = S.sessions.find((s) => s.key === key);
			if (sEntry) {
				sEntry.messageCount = history.length;
				sEntry.lastSeenMessageCount = history.length;
				sEntry._localUnread = false;
			}
			sessionStore.switchInProgress.value = false;
			S.setSessionSwitchInProgress(false);
			postHistoryLoadActions(key, searchContext, msgEls);
			if (S.chatInput) S.chatInput.focus();
		} else {
			sessionStore.switchInProgress.value = false;
			S.setSessionSwitchInProgress(false);
			if (S.chatInput) S.chatInput.focus();
		}
	});
}
