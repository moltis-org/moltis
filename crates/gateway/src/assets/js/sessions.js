// ── Sessions: list, switch, status helpers ──────────────────

import {
	appendChannelFooter,
	chatAddMsg,
	highlightAndScroll,
	removeThinking,
	scrollChatToBottom,
	stripChannelPrefix,
	updateTokenBar,
} from "./chat-ui.js";
import { formatTokens, renderMarkdown, sendRpc } from "./helpers.js";
import { makeChatIcon, makeCronIcon, makeTelegramIcon } from "./icons.js";
import { updateSessionProjectSelect } from "./project-combo.js";
import { currentPrefix, navigate, sessionPath } from "./router.js";
import { updateSandboxImageUI, updateSandboxUI } from "./sandbox.js";
import * as S from "./state.js";
import { confirmDialog } from "./ui.js";

// ── Fetch & render ──────────────────────────────────────────

export function fetchSessions() {
	sendRpc("sessions.list", {}).then((res) => {
		if (!res?.ok) return;
		S.setSessions(res.payload || []);
		renderSessionList();
		updateChatSessionHeader();
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

function isTelegramSession(s) {
	if (s.key.startsWith("telegram:")) return true;
	if (!s.channelBinding) return false;
	try {
		return JSON.parse(s.channelBinding).channel_type === "telegram";
	} catch (_e) {
		return false;
	}
}

function createSessionIcon(s) {
	var iconWrap = document.createElement("span");
	iconWrap.className = "session-icon";
	var telegram = isTelegramSession(s);
	var cron = s.key.startsWith("cron:");
	var icon = cron ? makeCronIcon() : telegram ? makeTelegramIcon() : makeChatIcon();
	iconWrap.appendChild(icon);
	if (telegram) {
		iconWrap.style.color = s.activeChannel ? "var(--accent)" : "var(--muted)";
		iconWrap.style.opacity = s.activeChannel ? "1" : "0.5";
		iconWrap.title = s.activeChannel ? "Active Telegram session" : "Telegram session (inactive)";
	} else {
		iconWrap.style.color = "var(--muted)";
	}
	var spinner = document.createElement("span");
	spinner.className = "session-spinner";
	iconWrap.appendChild(spinner);
	return iconWrap;
}

function createSessionMeta(s) {
	var meta = document.createElement("div");
	meta.className = "session-meta";
	meta.setAttribute("data-session-key", s.key);
	var count = s.messageCount || 0;
	var metaText = `${count} msg${count !== 1 ? "s" : ""}`;
	if (s.worktree_branch) {
		metaText += ` \u00b7 \u2387 ${s.worktree_branch}`;
	}
	meta.textContent = metaText;
	if (s.updatedAt) {
		var sep = document.createTextNode(" \u00b7 ");
		var t = document.createElement("time");
		t.setAttribute("data-epoch-ms", String(s.updatedAt));
		t.textContent = new Date(s.updatedAt).toISOString();
		meta.appendChild(sep);
		meta.appendChild(t);
	}
	return meta;
}

function createSessionActions() {
	var actions = document.createElement("div");
	actions.className = "session-actions";
	return actions;
}

export function renderSessionList() {
	var sessionList = S.$("sessionList");
	sessionList.textContent = "";
	var filtered = S.sessions;
	if (S.projectFilterId) {
		filtered = S.sessions.filter((s) => s.projectId === S.projectFilterId);
	}
	var tpl = document.getElementById("tpl-session-item");
	filtered.forEach((s) => {
		var frag = tpl.content.cloneNode(true);
		var item = frag.firstElementChild;
		item.className = `session-item${s.key === S.activeSessionKey ? " active" : ""}`;
		item.setAttribute("data-session-key", s.key);

		var iconWrap = item.querySelector(".session-icon");
		iconWrap.replaceWith(createSessionIcon(s));

		item.querySelector("[data-label-text]").textContent = s.label || s.key;

		var meta = item.querySelector(".session-meta");
		var newMeta = createSessionMeta(s);
		meta.replaceWith(newMeta);

		var actionsSlot = item.querySelector(".session-actions");
		actionsSlot.replaceWith(createSessionActions());

		item.addEventListener("click", () => {
			if (currentPrefix !== "/chats") {
				navigate(sessionPath(s.key));
			} else {
				switchSession(s.key);
			}
		});

		sessionList.appendChild(item);
	});
}

// ── Braille spinner for active sessions ─────────────────────
var spinnerFrames = [
	"\u280B",
	"\u2819",
	"\u2839",
	"\u2838",
	"\u283C",
	"\u2834",
	"\u2826",
	"\u2827",
	"\u2807",
	"\u280F",
];
var spinnerIndex = 0;
setInterval(() => {
	spinnerIndex = (spinnerIndex + 1) % spinnerFrames.length;
	var els = document.querySelectorAll(".session-item.replying .session-spinner");
	for (var el of els) el.textContent = spinnerFrames[spinnerIndex];
}, 80);

// ── Status helpers ──────────────────────────────────────────

export function setSessionReplying(key, replying) {
	var sessionList = S.$("sessionList");
	var el = sessionList.querySelector(`.session-item[data-session-key="${key}"]`);
	if (el) el.classList.toggle("replying", replying);
}

export function setSessionUnread(key, unread) {
	var sessionList = S.$("sessionList");
	var el = sessionList.querySelector(`.session-item[data-session-key="${key}"]`);
	if (el) el.classList.toggle("unread", unread);
}

export function bumpSessionCount(key, increment) {
	var sessionList = S.$("sessionList");
	var el = sessionList.querySelector(`.session-meta[data-session-key="${key}"]`);
	if (!el) return;
	var current = parseInt(el.textContent, 10) || 0;
	var next = current + increment;
	el.textContent = `${next} msg${next !== 1 ? "s" : ""}`;
}

// ── New session button ──────────────────────────────────────
var newSessionBtn = S.$("newSessionBtn");
newSessionBtn.addEventListener("click", () => {
	var key = `session:${crypto.randomUUID()}`;
	navigate(sessionPath(key));
});

// ── Switch session ──────────────────────────────────────────

function restoreSessionState(entry, projectId) {
	var effectiveProjectId = entry.projectId || projectId || "";
	S.setActiveProjectId(effectiveProjectId);
	localStorage.setItem("moltis-project", S.activeProjectId);
	updateSessionProjectSelect(S.activeProjectId);
	if (entry.model) {
		S.setSelectedModelId(entry.model);
		localStorage.setItem("moltis-model", entry.model);
		var found = S.models.find((m) => m.id === entry.model);
		if (S.modelComboLabel) S.modelComboLabel.textContent = found ? found.displayName || found.id : entry.model;
	}
	updateSandboxUI(entry.sandbox_enabled !== false);
	updateSandboxImageUI(entry.sandbox_image || null);
	updateChatSessionHeader();
}

function renderHistoryUserMessage(msg) {
	var userContent = msg.content || "";
	if (msg.channel) userContent = stripChannelPrefix(userContent);
	var userEl = chatAddMsg("user", renderMarkdown(userContent), true);
	if (userEl && msg.channel) appendChannelFooter(userEl, msg.channel);
	return userEl;
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
	var el = chatAddMsg("assistant", renderMarkdown(msg.content || ""), true);
	if (el && msg.model) {
		el.appendChild(createModelFooter(msg));
	}
	if (msg.inputTokens || msg.outputTokens) {
		S.sessionTokens.input += msg.inputTokens || 0;
		S.sessionTokens.output += msg.outputTokens || 0;
	}
	return el;
}

export function appendLastMessageTimestamp(epochMs) {
	if (!S.chatMsgBox) return;
	// Remove any previous last-message timestamp
	var old = S.chatMsgBox.querySelector(".msg-footer-time");
	if (old) old.remove();
	var lastMsg = S.chatMsgBox.lastElementChild;
	if (!lastMsg) return;
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

function postHistoryLoadActions(key, searchContext, msgEls, sessionList) {
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

	var item = sessionList.querySelector(`.session-item[data-session-key="${key}"]`);
	if (item?.classList.contains("replying") && S.chatMsgBox) {
		removeThinking();
		var thinkEl = document.createElement("div");
		thinkEl.className = "msg assistant thinking";
		thinkEl.id = "thinkingIndicator";
		thinkEl.appendChild(makeThinkingDots());
		S.chatMsgBox.appendChild(thinkEl);
		scrollChatToBottom();
	}
	if (!sessionList.querySelector(`.session-meta[data-session-key="${key}"]`)) {
		fetchSessions();
	}
}

function nextSessionKey(currentKey) {
	var idx = S.sessions.findIndex((x) => x.key === currentKey);
	if (idx >= 0 && idx + 1 < S.sessions.length) return S.sessions[idx + 1].key;
	if (idx > 0) return S.sessions[idx - 1].key;
	return "main";
}

export function updateChatSessionHeader() {
	var nameEl = S.$("chatSessionName");
	var inputEl = S.$("chatSessionRenameInput");
	var deleteBtn = S.$("chatSessionDelete");
	if (!nameEl) return;

	var s = S.sessions.find((x) => x.key === S.activeSessionKey);
	var fullName = s ? s.label || s.key : S.activeSessionKey;
	nameEl.textContent = fullName.length > 20 ? `${fullName.slice(0, 20)}\u2026` : fullName;
	nameEl.dataset.fullName = fullName;

	var isMain = S.activeSessionKey === "main";
	var isChannel = s?.channelBinding || S.activeSessionKey.startsWith("telegram:");
	var isCron = S.activeSessionKey.startsWith("cron:");
	var canRename = !(isMain || isChannel || isCron);

	nameEl.style.cursor = canRename ? "pointer" : "default";
	nameEl.title = canRename ? "Click to rename" : "";
	nameEl.onclick = canRename
		? () => {
				inputEl.style.width = `${nameEl.offsetWidth + 16}px`;
				nameEl.classList.add("hidden");
				inputEl.classList.remove("hidden");
				inputEl.value = nameEl.dataset.fullName || nameEl.textContent;
				inputEl.focus();
				inputEl.select();
			}
		: null;

	if (inputEl) {
		var commitRename = () => {
			var val = inputEl.value.trim();
			inputEl.classList.add("hidden");
			nameEl.classList.remove("hidden");
			if (val && val !== (nameEl.dataset.fullName || nameEl.textContent)) {
				sendRpc("sessions.patch", { key: S.activeSessionKey, label: val }).then((res) => {
					if (res?.ok) {
						nameEl.textContent = val;
						fetchSessions();
					}
				});
			}
		};
		inputEl.onblur = commitRename;
		inputEl.onkeydown = (e) => {
			if (e.key === "Enter") {
				e.preventDefault();
				inputEl.blur();
			}
			if (e.key === "Escape") {
				inputEl.value = nameEl.dataset.fullName || nameEl.textContent;
				inputEl.blur();
			}
		};
	}

	if (deleteBtn) {
		deleteBtn.classList.toggle("hidden", isMain || isCron);
		deleteBtn.onclick = () => {
			var msgCount = s ? s.messageCount || 0 : 0;
			var nextKey = nextSessionKey(S.activeSessionKey);
			var doDelete = () => {
				sendRpc("sessions.delete", { key: S.activeSessionKey }).then((res) => {
					if (res && !res.ok && res.error && res.error.indexOf("uncommitted changes") !== -1) {
						confirmDialog("Worktree has uncommitted changes. Force delete?").then((yes) => {
							if (!yes) return;
							sendRpc("sessions.delete", { key: S.activeSessionKey, force: true }).then(() => {
								switchSession(nextKey);
								fetchSessions();
							});
						});
						return;
					}
					switchSession(nextKey);
					fetchSessions();
				});
			};
			if (msgCount > 0) {
				confirmDialog("Delete this session?").then((yes) => {
					if (yes) doDelete();
				});
			} else {
				doDelete();
			}
		};
	}
}

export function switchSession(key, searchContext, projectId) {
	var sessionList = S.$("sessionList");
	S.setActiveSessionKey(key);
	localStorage.setItem("moltis-session", key);
	history.replaceState(null, "", sessionPath(key));
	if (S.chatMsgBox) S.chatMsgBox.textContent = "";
	S.setStreamEl(null);
	S.setStreamText("");
	S.setLastHistoryIndex(-1);
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionContextWindow(0);
	updateTokenBar();

	var items = sessionList.querySelectorAll(".session-item");
	items.forEach((el) => {
		var isTarget = el.getAttribute("data-session-key") === key;
		el.classList.toggle("active", isTarget);
		if (isTarget) el.classList.remove("unread");
	});

	S.setSessionSwitchInProgress(true);
	var switchParams = { key: key };
	if (projectId) switchParams.project_id = projectId;
	sendRpc("sessions.switch", switchParams).then((res) => {
		if (res?.ok && res.payload) {
			var entry = res.payload.entry || {};
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
				} else {
					msgEls.push(null);
				}
			});
			S.setChatBatchLoading(false);
			S.setLastHistoryIndex(history.length > 0 ? history.length - 1 : -1);
			if (history.length > 0) {
				var lastMsg = history[history.length - 1];
				var ts = lastMsg.created_at;
				if (ts) appendLastMessageTimestamp(ts);
			}
			S.setSessionSwitchInProgress(false);
			postHistoryLoadActions(key, searchContext, msgEls, sessionList);
		} else {
			S.setSessionSwitchInProgress(false);
		}
	});
}
