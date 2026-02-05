// ── WebSocket ─────────────────────────────────────────────────

import {
	appendChannelFooter,
	chatAddErrorCard,
	chatAddErrorMsg,
	chatAddMsg,
	removeThinking,
	renderApprovalCard,
	stripChannelPrefix,
	updateTokenBar,
} from "./chat-ui.js";
import { eventListeners } from "./events.js";
import { formatTokens, nextId, renderMarkdown, sendRpc } from "./helpers.js";
import { clearLogsAlert, updateLogsAlert } from "./logs-alert.js";
import { fetchModels } from "./models.js";
import { prefetchChannels } from "./page-channels.js";
import { renderCompactCard } from "./page-chat.js";
import { fetchProjects } from "./projects.js";
import { currentPage, currentPrefix, mount } from "./router.js";
import {
	appendLastMessageTimestamp,
	bumpSessionCount,
	fetchSessions,
	setSessionReplying,
	setSessionUnread,
	switchSession,
} from "./sessions.js";
import * as S from "./state.js";

// ── Chat event handlers ──────────────────────────────────────

function makeThinkingDots() {
	var tpl = document.getElementById("tpl-thinking-dots");
	return tpl.content.cloneNode(true).firstElementChild;
}

function handleChatThinking(_p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	removeThinking();
	var thinkEl = document.createElement("div");
	thinkEl.className = "msg assistant thinking";
	thinkEl.id = "thinkingIndicator";
	thinkEl.appendChild(makeThinkingDots());
	S.chatMsgBox.appendChild(thinkEl);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

function handleChatThinkingText(p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	var indicator = document.getElementById("thinkingIndicator");
	if (indicator) {
		while (indicator.firstChild) indicator.removeChild(indicator.firstChild);
		var textEl = document.createElement("span");
		textEl.className = "thinking-text";
		textEl.textContent = p.text;
		indicator.appendChild(textEl);
		S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
	}
}

function handleChatThinkingDone(_p, isActive, isChatPage) {
	if (isActive && isChatPage) removeThinking();
}

/** Build a short summary string for a tool call card. */
function toolCallSummary(name, args) {
	if (!args) return name || "tool";
	switch (name) {
		case "exec":
			return args.command || "exec";
		case "web_fetch":
			return `web_fetch ${args.url || ""}`.trim();
		case "web_search":
			return `web_search "${args.query || ""}"`;
		default:
			return name || "tool";
	}
}

function handleChatToolCallStart(p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	removeThinking();
	var tpl = document.getElementById("tpl-exec-card");
	var frag = tpl.content.cloneNode(true);
	var card = frag.firstElementChild;
	card.id = `tool-${p.toolCallId}`;
	var cmd = toolCallSummary(p.toolName, p.arguments);
	card.querySelector("[data-cmd]").textContent = ` ${cmd}`;
	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

function appendToolResult(toolCard, result) {
	var out = (result.stdout || "").replace(/\n+$/, "");
	S.setLastToolOutput(out);
	if (out) {
		var outEl = document.createElement("pre");
		outEl.className = "exec-output";
		outEl.textContent = out;
		toolCard.appendChild(outEl);
	}
	var stderrText = (result.stderr || "").replace(/\n+$/, "");
	if (stderrText) {
		var errEl = document.createElement("pre");
		errEl.className = "exec-output exec-stderr";
		errEl.textContent = stderrText;
		toolCard.appendChild(errEl);
	}
	if (result.exit_code !== undefined && result.exit_code !== 0) {
		var codeEl = document.createElement("div");
		codeEl.className = "exec-exit";
		codeEl.textContent = `exit ${result.exit_code}`;
		toolCard.appendChild(codeEl);
	}
}

function handleChatToolCallEnd(p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	var toolCard = document.getElementById(`tool-${p.toolCallId}`);
	if (!toolCard) return;
	toolCard.className = `msg exec-card ${p.success ? "exec-ok" : "exec-err"}`;
	var toolSpin = toolCard.querySelector(".exec-status");
	if (toolSpin) toolSpin.remove();
	if (p.success && p.result) {
		appendToolResult(toolCard, p.result);
	} else if (!p.success && p.error && p.error.detail) {
		var errMsg = document.createElement("div");
		errMsg.className = "exec-error-detail";
		errMsg.textContent = p.error.detail;
		toolCard.appendChild(errMsg);
	}
}

function handleChatChannelUser(p, _isActive, isChatPage) {
	if (!isChatPage) return;
	if (p.sessionKey && p.sessionKey !== S.activeSessionKey) {
		switchSession(p.sessionKey);
	}
	var active = p.sessionKey ? p.sessionKey === S.activeSessionKey : p.sessionKey === undefined;
	if (!active) return;
	if (p.messageIndex !== undefined && p.messageIndex <= S.lastHistoryIndex) return;
	var cleanText = stripChannelPrefix(p.text || "");
	var el = chatAddMsg("user", renderMarkdown(cleanText), true);
	if (el && p.channel) {
		appendChannelFooter(el, p.channel);
	}
}

// Safe: renderMarkdown calls esc() first — all user input is HTML-escaped before
// being passed to innerHTML. This is the standard rendering path for chat messages.
function setSafeMarkdownHtml(el, text) {
	el.innerHTML = renderMarkdown(text); // eslint-disable-line no-unsanitized/property
}

function handleChatDelta(p, isActive, isChatPage) {
	if (!(p.text && isActive && isChatPage)) return;
	removeThinking();
	if (!S.streamEl) {
		S.setStreamText("");
		S.setStreamEl(document.createElement("div"));
		S.streamEl.className = "msg assistant";
		S.chatMsgBox.appendChild(S.streamEl);
	}
	S.setStreamText(S.streamText + p.text);
	setSafeMarkdownHtml(S.streamEl, S.streamText);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

function resolveFinalMessageEl(p) {
	var isEcho =
		S.lastToolOutput &&
		p.text &&
		p.text.replace(/[`\s]/g, "").indexOf(S.lastToolOutput.replace(/\s/g, "").substring(0, 80)) !== -1;
	if (!isEcho) {
		if (p.text && S.streamEl) {
			setSafeMarkdownHtml(S.streamEl, p.text);
			return S.streamEl;
		}
		if (p.text) return chatAddMsg("assistant", renderMarkdown(p.text), true);
		return null;
	}
	if (S.streamEl) S.streamEl.remove();
	return null;
}

function appendFinalFooter(msgEl, p) {
	if (!(msgEl && p.model)) return;
	var footer = document.createElement("div");
	footer.className = "msg-model-footer";
	var footerText = p.provider ? `${p.provider} / ${p.model}` : p.model;
	if (p.inputTokens || p.outputTokens) {
		footerText += ` \u00b7 ${formatTokens(p.inputTokens || 0)} in / ${formatTokens(p.outputTokens || 0)} out`;
	}
	footer.textContent = footerText;
	msgEl.appendChild(footer);
}

function handleChatFinal(p, isActive, isChatPage, eventSession) {
	if (p.messageIndex !== undefined && p.messageIndex <= S.lastHistoryIndex) {
		setSessionReplying(eventSession, false);
		return;
	}
	bumpSessionCount(eventSession, 1);
	setSessionReplying(eventSession, false);
	if (!isActive) {
		setSessionUnread(eventSession, true);
	}
	if (!(isActive && isChatPage)) return;
	removeThinking();
	var msgEl = resolveFinalMessageEl(p);
	appendFinalFooter(msgEl, p);
	if (p.inputTokens || p.outputTokens) {
		S.sessionTokens.input += p.inputTokens || 0;
		S.sessionTokens.output += p.outputTokens || 0;
		updateTokenBar();
	}
	appendLastMessageTimestamp(Date.now());
	S.setStreamEl(null);
	S.setStreamText("");
	S.setLastToolOutput("");
}

function handleChatAutoCompact(p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	if (p.phase === "start") {
		chatAddMsg("system", "Compacting conversation (context limit reached)\u2026");
	} else if (p.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		renderCompactCard(p);
		S.setSessionTokens({ input: 0, output: 0 });
		updateTokenBar();
	} else if (p.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Auto-compact failed: ${p.error || "unknown error"}`);
	}
}

function handleChatError(p, isActive, isChatPage, eventSession) {
	setSessionReplying(eventSession, false);
	if (!(isActive && isChatPage)) return;
	removeThinking();
	if (p.error?.title) {
		chatAddErrorCard(p.error);
	} else {
		chatAddErrorMsg(p.message || "unknown");
	}
	S.setStreamEl(null);
	S.setStreamText("");
}

function handleChatNotice(p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	// Show notice message with title if provided
	var msg = p.title ? `**${p.title}:** ${p.message}` : p.message;
	chatAddMsg("system", msg);
}

var chatHandlers = {
	thinking: handleChatThinking,
	thinking_text: handleChatThinkingText,
	thinking_done: handleChatThinkingDone,
	tool_call_start: handleChatToolCallStart,
	tool_call_end: handleChatToolCallEnd,
	channel_user: handleChatChannelUser,
	delta: handleChatDelta,
	final: handleChatFinal,
	auto_compact: handleChatAutoCompact,
	error: handleChatError,
	notice: handleChatNotice,
};

function handleChatEvent(p) {
	var eventSession = p.sessionKey || S.activeSessionKey;
	var isActive = eventSession === S.activeSessionKey;
	var isChatPage = currentPrefix === "/chats";

	if (isActive && S.sessionSwitchInProgress) return;

	if (p.sessionKey && !S.sessions.find((s) => s.key === p.sessionKey)) {
		fetchSessions();
	}

	var handler = chatHandlers[p.state];
	if (handler) handler(p, isActive, isChatPage, eventSession);
}

function handleApprovalEvent(payload) {
	renderApprovalCard(payload.requestId, payload.command);
}

function handleLogEntry(payload) {
	if (S.logsEventHandler) S.logsEventHandler(payload);
	if (currentPage !== "/logs") {
		var ll = (payload.level || "").toUpperCase();
		if (ll === "ERROR") {
			S.setUnseenErrors(S.unseenErrors + 1);
			updateLogsAlert();
		} else if (ll === "WARN") {
			S.setUnseenWarns(S.unseenWarns + 1);
			updateLogsAlert();
		}
	}
}

function handleSandboxImageBuild(payload) {
	var isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (payload.phase === "start") {
		chatAddMsg("system", "Building sandbox image (installing packages)\u2026");
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		var msg = payload.built ? `Sandbox image ready: ${payload.tag}` : `Sandbox image already cached: ${payload.tag}`;
		chatAddMsg("system", msg);
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Sandbox image build failed: ${payload.error || "unknown"}`);
	}
}

function handleSandboxImageProvision(payload) {
	var isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (payload.phase === "start") {
		chatAddMsg("system", "Provisioning sandbox packages\u2026");
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("system", "Sandbox packages provisioned");
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Sandbox provisioning failed: ${payload.error || "unknown"}`);
	}
}

var eventHandlers = {
	chat: handleChatEvent,
	"exec.approval.requested": handleApprovalEvent,
	"logs.entry": handleLogEntry,
	"sandbox.image.build": handleSandboxImageBuild,
	"sandbox.image.provision": handleSandboxImageProvision,
};

function dispatchFrame(frame) {
	if (frame.type === "res") {
		var cb = S.pending[frame.id];
		if (cb) {
			delete S.pending[frame.id];
			cb(frame);
		}
		return;
	}
	if (frame.type === "event") {
		var listeners = eventListeners[frame.event] || [];
		listeners.forEach((h) => {
			h(frame.payload || {});
		});
		var handler = eventHandlers[frame.event];
		if (handler) handler(frame.payload || {});
	}
}

export function connect() {
	setStatus("connecting", "connecting...");
	var proto = location.protocol === "https:" ? "wss:" : "ws:";
	S.setWs(new WebSocket(`${proto}//${location.host}/ws`));

	S.ws.onopen = () => {
		var id = nextId();
		S.ws.send(
			JSON.stringify({
				type: "req",
				id: id,
				method: "connect",
				params: {
					minProtocol: 3,
					maxProtocol: 3,
					client: {
						id: "web-chat-ui",
						version: "0.1.0",
						platform: "browser",
						mode: "operator",
					},
				},
			}),
		);
		S.pending[id] = (frame) => {
			var hello = frame.ok && frame.payload;
			if (hello && hello.type === "hello-ok") {
				S.setConnected(true);
				S.setReconnectDelay(1000);
				var assetHash = document.querySelector('meta[name="build-ts"]')?.content || "?";
				setStatus("connected", `connected (v${hello.protocol}) assets:${assetHash.substring(0, 8)}`);
				var now = new Date();
				var ts = now.toLocaleTimeString([], {
					hour: "2-digit",
					minute: "2-digit",
					second: "2-digit",
				});
				chatAddMsg("system", `Connected to moltis gateway v${hello.server.version} at ${ts}`);
				fetchModels();
				fetchSessions();
				fetchProjects();
				prefetchChannels();
				sendRpc("logs.status", {}).then((res) => {
					if (res?.ok) {
						var p = res.payload || {};
						S.setUnseenErrors(p.unseen_errors || 0);
						S.setUnseenWarns(p.unseen_warns || 0);
						if (currentPage === "/logs") clearLogsAlert();
						else updateLogsAlert();
					}
				});
				if (currentPage === "/chats" || currentPrefix === "/chats") mount(currentPage);
			} else {
				setStatus("", "handshake failed");
				var reason = frame.error?.message || "unknown error";
				chatAddMsg("error", `Handshake failed: ${reason}`);
			}
		};
	};

	S.ws.onmessage = (evt) => {
		var frame;
		try {
			frame = JSON.parse(evt.data);
		} catch (_e) {
			return;
		}
		dispatchFrame(frame);
	};

	S.ws.onclose = () => {
		var wasConnected = S.connected;
		S.setConnected(false);
		if (wasConnected) {
			setStatus("", "disconnected \u2014 reconnecting\u2026");
		}
		S.setStreamEl(null);
		S.setStreamText("");
		// Reject all pending RPC callbacks so callers don't hang forever.
		for (var id in S.pending) {
			S.pending[id]({ ok: false, error: { message: "WebSocket disconnected" } });
			delete S.pending[id];
		}
		scheduleReconnect();
	};

	S.ws.onerror = () => {
		/* handled by onclose */
	};
}

function setStatus(state, text) {
	var dot = S.$("statusDot");
	var sText = S.$("statusText");
	dot.className = `status-dot ${state}`;
	sText.textContent = text;
	var sendBtn = S.$("sendBtn");
	if (sendBtn) sendBtn.disabled = state !== "connected";
}

var reconnectTimer = null;

function scheduleReconnect() {
	if (reconnectTimer) return;
	reconnectTimer = setTimeout(() => {
		reconnectTimer = null;
		S.setReconnectDelay(Math.min(S.reconnectDelay * 1.5, 5000));
		connect();
	}, S.reconnectDelay);
}

document.addEventListener("visibilitychange", () => {
	if (!(document.hidden || S.connected)) {
		clearTimeout(reconnectTimer);
		reconnectTimer = null;
		S.setReconnectDelay(1000);
		connect();
	}
});
