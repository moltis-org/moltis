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
	formatTokenSpeed,
	formatTokens,
	parseAgentsListPayload,
	renderAudioPlayer,
	renderMarkdown,
	renderScreenshot,
	sendRpc,
	tokenSpeedTone,
	toolCallSummary,
} from "./helpers.js";
import { attachMessageVoiceControl } from "./message-voice.js";
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
import { sessionStore } from "./stores/session-store.js";
import { confirmDialog } from "./ui.js";

var SESSION_PREVIEW_MAX_CHARS = 200;
var switchRequestSeq = 0;
var latestSwitchRequestBySession = new Map();

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
			S.setSessionCurrentInputTokens(0);
			updateTokenBar();
			var activeKey = sessionStore.activeSessionKey.value || S.activeSessionKey;
			var session = sessionStore.getByKey(activeKey);
			if (session) {
				session.syncCounts(0, 0);
				session.replying.value = false;
				session.activeRunId.value = null;
			}
			clearSessionHistory(activeKey);
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
		(s) =>
			s.key !== "main" &&
			!s.key.startsWith("cron:") &&
			!s.key.startsWith("telegram:") &&
			!s.key.startsWith("msteams:") &&
			!s.channelBinding,
	);
	clearAllBtn.classList.toggle("hidden", !hasClearable);
}

if (clearAllBtn) {
	clearAllBtn.addEventListener("click", () => {
		var allSessions = sessionStore.sessions.value;
		var count = allSessions.filter(
			(s) =>
				s.key !== "main" &&
				!s.key.startsWith("cron:") &&
				!s.key.startsWith("telegram:") &&
				!s.key.startsWith("msteams:") &&
				!s.channelBinding,
		).length;
		if (count === 0) return;
		confirmDialog(
			`Delete ${count} session${count !== 1 ? "s" : ""}? Main, channel-bound, and cron sessions will be kept.`,
		).then((yes) => {
			if (!yes) return;
			clearAllBtn.disabled = true;
			clearAllBtn.textContent = "Clearing\u2026";
			sendRpc("sessions.clear_all", {}).then((res) => {
				clearAllBtn.disabled = false;
				clearAllBtn.textContent = "Clear";
				if (res?.ok) {
					clearSessionHistory();
					// If the active session was deleted, switch to main.
					var active = sessionStore.getByKey(sessionStore.activeSessionKey.value);
					var wasKept =
						!active ||
						active.key === "main" ||
						active.key.startsWith("cron:") ||
						active.key.startsWith("telegram:") ||
						active.key.startsWith("msteams:") ||
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
	var sandboxRuntimeAvailable = (S.sandboxInfo?.backend || "none") !== "none";
	var effectiveSandboxRoute = entry.sandbox_enabled !== false && sandboxRuntimeAvailable;
	S.setSessionExecMode(effectiveSandboxRoute ? "sandbox" : "host");
	S.setSessionExecPromptSymbol(effectiveSandboxRoute || S.hostExecIsRoot ? "#" : "$");
	updateCommandInputUI();
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
		ftText += ` \u00b7 ${formatTokens(msg.inputTokens || 0)} in / ${formatTokens(msg.outputTokens || 0)} out`;
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

function postHistoryLoadActions(key, searchContext, msgEls, thinkingText) {
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
		if (thinkingText) {
			var textEl = document.createElement("span");
			textEl.className = "thinking-text";
			textEl.textContent = thinkingText;
			thinkEl.appendChild(textEl);
		} else {
			thinkEl.appendChild(makeThinkingDots());
		}
		S.chatMsgBox.appendChild(thinkEl);
		scrollChatToBottom();
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
		S.setSessions([...S.sessions, created]);
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

function syncHistoryState(key, history, historyTailIndex) {
	var count = Array.isArray(history) ? history.length : 0;
	var sessionEntry = sessionStore.getByKey(key);
	if (sessionEntry) {
		sessionEntry.syncCounts(count, count);
		sessionEntry.localUnread.value = false;
		sessionEntry.lastHistoryIndex.value = historyTailIndex;
	}
	var legacy = S.sessions.find((s) => s.key === key);
	if (legacy) {
		legacy.messageCount = count;
		legacy.lastSeenMessageCount = count;
		legacy._localUnread = false;
	}
	S.setLastHistoryIndex(historyTailIndex);
}

function renderHistory(key, history, searchContext, thinkingText) {
	hideSessionLoadIndicator();
	if (S.chatMsgBox) S.chatMsgBox.textContent = "";
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
	syncHistoryState(key, history, historyTailIndex);

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
	postHistoryLoadActions(key, searchContext, msgEls, thinkingText);
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
	var cachedHistory = getSessionHistory(key);
	var hasCache = Array.isArray(cachedHistory);
	var cacheRevisionAtRequest = getHistoryRevision(key);
	startSessionRefresh(key, !hasCache);
	if (hasCache) {
		renderHistory(key, cachedHistory, searchContext, null);
	} else {
		showSessionLoadIndicator();
	}

	sendRpc("sessions.switch", switchParams)
		.then((res) => {
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
			var serverHistory = Array.isArray(res.payload.history) ? res.payload.history : [];
			var appliedServerHistory = false;
			if (shouldApplyServerHistory(key, serverHistory, cacheRevisionAtRequest)) {
				replaceSessionHistory(key, serverHistory);
				appliedServerHistory = true;
			}
			var history = getSessionHistory(key) || serverHistory;
			if (stillActive) {
				restoreSessionState(entry, projectId);
				applyReplyingStateFromSwitchPayload(key, res.payload);
				var thinkingText = res.payload.replying ? res.payload.thinkingText || null : null;
				var shouldRerender = !hasCache || Boolean(searchContext?.query) || appliedServerHistory;
				if (shouldRerender) {
					renderHistory(key, history, searchContext, thinkingText);
				} else {
					postHistoryLoadActions(key, searchContext, [], thinkingText);
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
