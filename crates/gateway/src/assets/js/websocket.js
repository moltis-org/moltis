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
import {
	formatTokens,
	renderAudioPlayer,
	renderMapLinks,
	renderMarkdown,
	renderScreenshot,
	sendRpc,
	toolCallSummary,
} from "./helpers.js";
import { clearLogsAlert, updateLogsAlert } from "./logs-alert.js";
import { fetchModels } from "./models.js";
import { prefetchChannels } from "./page-channels.js";
import { maybeRefreshFullContext, renderCompactCard } from "./page-chat.js";
import { fetchProjects } from "./projects.js";
import { currentPage, currentPrefix, mount } from "./router.js";
import {
	appendLastMessageTimestamp,
	bumpSessionCount,
	fetchSessions,
	setSessionReplying,
	setSessionUnread,
} from "./sessions.js";
import * as S from "./state.js";
import { sessionStore } from "./stores/session-store.js";
import { connectWs, forceReconnect } from "./ws-connect.js";

// ── Chat event handlers ──────────────────────────────────────

var ttsWebStatus = null; // null = unknown, true/false = enabled state

async function appendAssistantVoiceIfEnabled(msgEl, text) {
	if (!(msgEl && text)) return false;

	if (ttsWebStatus === null) {
		var status = await sendRpc("tts.status", {});
		ttsWebStatus = status?.ok && status.payload?.enabled === true;
	}
	if (!ttsWebStatus) return false;

	var tts = await sendRpc("tts.convert", { text: text, format: "ogg" });
	if (!(tts?.ok && tts.payload?.audio)) {
		if (tts?.error) {
			console.warn("TTS convert failed:", tts.error.message || tts.error);
		}
		return false;
	}

	msgEl.textContent = "";

	var mimeType = tts.payload.mimeType || "audio/ogg";
	var src = `data:${mimeType};base64,${tts.payload.audio}`;
	renderAudioPlayer(msgEl, src, true);
	return true;
}

function makeThinkingDots() {
	var tpl = document.getElementById("tpl-thinking-dots");
	return tpl.content.cloneNode(true).firstElementChild;
}

function moveFirstQueuedToChat() {
	var tray = document.getElementById("queuedMessages");
	if (!tray) return;
	var firstQueued = tray.querySelector(".msg.user.queued");
	if (!firstQueued) return;
	console.debug("[queued] moving queued message from tray to chat", {
		remaining: tray.querySelectorAll(".msg").length - 1,
	});
	firstQueued.classList.remove("queued");
	var badge = firstQueued.querySelector(".queued-badge");
	if (badge) badge.remove();
	S.chatMsgBox.appendChild(firstQueued);
	if (!tray.querySelector(".msg")) tray.classList.add("hidden");
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

function handleChatVoicePending(_p, isActive, isChatPage, eventSession) {
	// Update per-session signal
	var session = sessionStore.getByKey(eventSession);
	if (session) session.voicePending.value = true;
	if (!(isActive && isChatPage)) return;
	// Dual-write to global state for backward compat
	S.setVoicePending(true);
	// Keep the existing thinking dots visible — no separate voice indicator.
}

function handleChatToolCallStart(p, isActive, isChatPage, eventSession) {
	// Update per-session signal
	var session = sessionStore.getByKey(eventSession);
	if (session) session.streamText.value = "";
	if (!(isActive && isChatPage)) return;
	removeThinking();
	// Close the current streaming element so new text deltas after this tool
	// call will create a fresh element positioned after the tool card
	if (S.streamEl) {
		S.setStreamEl(null);
		S.setStreamText("");
	}
	var tpl = document.getElementById("tpl-exec-card");
	var frag = tpl.content.cloneNode(true);
	var card = frag.firstElementChild;
	card.id = `tool-${p.toolCallId}`;
	var cmd = toolCallSummary(p.toolName, p.arguments, p.executionMode);
	card.querySelector("[data-cmd]").textContent = ` ${cmd}`;
	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

function appendToolResult(toolCard, result, eventSession) {
	var out = (result.stdout || "").replace(/\n+$/, "");
	// Update per-session signal
	var toolSession = sessionStore.getByKey(eventSession);
	if (toolSession) toolSession.lastToolOutput.value = out;
	// Dual-write to global state for backward compat
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
	// Browser screenshot support - display as thumbnail with lightbox and download
	if (result.screenshot) {
		var imgSrc = result.screenshot.startsWith("data:")
			? result.screenshot
			: `data:image/png;base64,${result.screenshot}`;
		renderScreenshot(toolCard, imgSrc, result.screenshot_scale || 1);
	}
	// Map link buttons (show_map tool)
	if (result.map_links) {
		renderMapLinks(toolCard, result.map_links, result.label);
	}
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Tool result processing with multiple cases
function handleChatToolCallEnd(p, isActive, isChatPage, eventSession) {
	// Always bump badge — the server persists a tool_result message for each call.
	bumpSessionCount(eventSession, 1);
	if (!(isActive && isChatPage)) return;
	var toolCard = document.getElementById(`tool-${p.toolCallId}`);
	if (!toolCard) return;

	// Check if this is a schema validation error (model sent malformed args)
	// These are expected sometimes and the agent retries automatically
	var isValidationError = false;
	if (!p.success && p.error && p.error.detail) {
		var errDetail = p.error.detail.toLowerCase();
		isValidationError =
			errDetail.includes("missing field") ||
			errDetail.includes("missing required") ||
			errDetail.includes("missing 'action'") ||
			errDetail.includes("missing 'url'");
	}

	// Use muted "retry" style for validation errors, normal styles otherwise
	if (isValidationError) {
		toolCard.className = "msg exec-card exec-retry";
	} else {
		toolCard.className = `msg exec-card ${p.success ? "exec-ok" : "exec-err"}`;
	}

	var toolSpin = toolCard.querySelector(".exec-status");
	if (toolSpin) toolSpin.remove();
	if (p.success && p.result) {
		appendToolResult(toolCard, p.result, eventSession);
	} else if (!p.success && p.error && p.error.detail) {
		var errMsg = document.createElement("div");
		errMsg.className = isValidationError ? "exec-retry-detail" : "exec-error-detail";
		errMsg.textContent = p.error.detail;
		toolCard.appendChild(errMsg);
	}
}

function handleChatChannelUser(p, isActive, isChatPage, eventSession) {
	// Always bump the badge so the total message count stays accurate,
	// even when the user is not on the chat page (e.g. Telegram messages).
	bumpSessionCount(eventSession, 1);
	if (!isActive) {
		setSessionUnread(eventSession, true);
	}
	if (!(isChatPage && isActive)) return;
	// Compare against the per-session history index, not the global one,
	// to avoid skipping events when viewing a different session.
	var chanSession = sessionStore.getByKey(p.sessionKey || S.activeSessionKey);
	var chanLastIdx = chanSession ? chanSession.lastHistoryIndex.value : S.lastHistoryIndex;
	if (p.messageIndex !== undefined && p.messageIndex <= chanLastIdx) return;
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

function handleChatDelta(p, isActive, isChatPage, eventSession) {
	if (!p.text) return;
	// Update per-session signal
	var session = sessionStore.getByKey(eventSession);
	if (session) session.streamText.value += p.text;
	if (!(isActive && isChatPage)) return;
	// When voice is pending, accumulate text silently without rendering.
	if (S.voicePending) {
		S.setStreamText(S.streamText + p.text);
		return;
	}
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

function normalizeEchoComparable(text) {
	if (!text) return "";
	return text
		.replace(/```[a-zA-Z0-9_-]*\n?/g, "")
		.replace(/```/g, "")
		.replace(/[`\s]/g, "");
}

function isPureToolOutputEcho(finalText, toolOutput) {
	var finalComparable = normalizeEchoComparable(finalText);
	var toolComparable = normalizeEchoComparable(toolOutput);
	if (!(finalComparable && toolComparable)) return false;
	return finalComparable === toolComparable;
}

function resolveFinalMessageEl(p) {
	var isEcho = isPureToolOutputEcho(p.text, S.lastToolOutput);
	if (!isEcho) {
		if (p.text && S.streamEl) {
			setSafeMarkdownHtml(S.streamEl, p.text);
			return S.streamEl;
		}
		if (p.text) return chatAddMsg("assistant", renderMarkdown(p.text), true);
		// No text (silent reply) — remove any leftover stream element.
		if (S.streamEl) S.streamEl.remove();
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
	if (p.replyMedium === "voice" || p.replyMedium === "text") {
		var badge = document.createElement("span");
		badge.className = "reply-medium-badge";
		badge.textContent = p.replyMedium;
		footer.appendChild(badge);
	}
	msgEl.appendChild(footer);
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Final message handling with audio/voice branching
function handleChatFinal(p, isActive, isChatPage, eventSession) {
	// Always bump badge — the server persists the final assistant message.
	bumpSessionCount(eventSession, 1);
	// Compare against the per-session history index so cross-session
	// events aren't wrongly skipped by another session's index.
	var evtSession = sessionStore.getByKey(eventSession);
	var lastIdx = evtSession ? evtSession.lastHistoryIndex.value : S.lastHistoryIndex;
	if (p.messageIndex !== undefined && p.messageIndex <= lastIdx) {
		setSessionReplying(eventSession, false);
		return;
	}
	setSessionReplying(eventSession, false);
	if (!isActive) {
		setSessionUnread(eventSession, true);
	}
	if (!(isActive && isChatPage)) {
		S.setVoicePending(false);
		return;
	}
	removeThinking();

	if (S.voicePending && p.text && p.replyMedium === "voice") {
		// Voice pending path: we suppressed streaming, so render everything at once.
		console.debug("[audio] voice-pending path, audio:", !!p.audio, "text:", p.text.substring(0, 40));
		var msgEl = S.streamEl || document.createElement("div");
		msgEl.className = "msg assistant";
		msgEl.textContent = "";
		if (!msgEl.parentNode) S.chatMsgBox.appendChild(msgEl);

		if (p.audio) {
			var filename = p.audio.split("/").pop();
			var audioSrc = `/api/sessions/${encodeURIComponent(p.sessionKey || S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			console.debug("[audio] rendering persisted audio:", filename);
			renderAudioPlayer(msgEl, audioSrc, true);
		}
		// Safe: renderMarkdown calls esc() first — all user input is HTML-escaped.
		var textWrap = document.createElement("div");
		textWrap.className = "mt-2";
		setSafeMarkdownHtml(textWrap, p.text);
		msgEl.appendChild(textWrap);
		appendFinalFooter(msgEl, p);
		S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
	} else {
		var resolvedEl = resolveFinalMessageEl(p);
		if (resolvedEl && p.text && p.replyMedium === "voice") {
			console.debug(
				"[audio] streamed path, audio:",
				!!p.audio,
				"voicePending:",
				S.voicePending,
				"text:",
				p.text.substring(0, 40),
			);
			if (p.audio) {
				var fn2 = p.audio.split("/").pop();
				var src2 = `/api/sessions/${encodeURIComponent(p.sessionKey || S.activeSessionKey)}/media/${encodeURIComponent(fn2)}`;
				console.debug("[audio] rendering persisted audio (streamed):", fn2);
				resolvedEl.textContent = "";
				renderAudioPlayer(resolvedEl, src2, true);
				appendFinalFooter(resolvedEl, p);
			} else {
				console.debug("[audio] no persisted audio, trying web TTS");
				appendAssistantVoiceIfEnabled(resolvedEl, p.text)
					.catch((err) => {
						console.warn("Web UI TTS playback failed:", err);
						return false;
					})
					.finally(() => appendFinalFooter(resolvedEl, p));
			}
		} else {
			// Silent reply — attach footer to the last visible assistant element
			// (e.g. exec card). Never attach to a user message.
			var target = resolvedEl;
			if (!target) {
				var last = S.chatMsgBox?.lastElementChild;
				if (last && !last.classList.contains("user")) target = last;
			}
			appendFinalFooter(target, p);
		}
	}
	if (p.inputTokens || p.outputTokens) {
		S.sessionTokens.input += p.inputTokens || 0;
		S.sessionTokens.output += p.outputTokens || 0;
		updateTokenBar();
	}
	appendLastMessageTimestamp(Date.now());
	// Reset per-session stream state
	var finalSession = sessionStore.getByKey(eventSession);
	if (finalSession) finalSession.resetStreamState();
	// Dual-write to global state for backward compat
	S.setStreamEl(null);
	S.setStreamText("");
	S.setLastToolOutput("");
	S.setVoicePending(false);
	maybeRefreshFullContext();
	// Move the next queued message from the tray AFTER the response is
	// fully rendered. This ensures correct ordering: user-msg → response →
	// next-user-msg → next-response (never next-user-msg before response).
	moveFirstQueuedToChat();
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
	// Reset per-session stream state
	var errSession = sessionStore.getByKey(eventSession);
	if (errSession) errSession.resetStreamState();
	if (!(isActive && isChatPage)) {
		S.setVoicePending(false);
		return;
	}
	removeThinking();
	if (p.error?.title) {
		chatAddErrorCard(p.error);
	} else {
		chatAddErrorMsg(p.message || "unknown");
	}
	S.setStreamEl(null);
	S.setStreamText("");
	S.setVoicePending(false);
	moveFirstQueuedToChat();
}

function handleChatNotice(p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	// Show notice message with title if provided
	var msg = p.title ? `**${p.title}:** ${p.message}` : p.message;
	chatAddMsg("system", msg);
}

function handleChatQueueCleared(_p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	var tray = document.getElementById("queuedMessages");
	if (tray) {
		var count = tray.querySelectorAll(".msg").length;
		console.debug("[queued] queue_cleared: removing all from tray", { count });
		while (tray.firstChild) tray.removeChild(tray.firstChild);
		tray.classList.add("hidden");
	}
}

function handleChatSessionCleared(_p, isActive, isChatPage, eventSession) {
	// Reset badge, unread state, and history index for every client.
	var session = sessionStore.getByKey(eventSession);
	if (session) {
		session.syncCounts(0, 0);
		session.lastHistoryIndex.value = -1;
	}
	if (!(isActive && isChatPage)) return;
	// Active viewer: clear the chat box and token bar.
	if (S.chatMsgBox) S.chatMsgBox.textContent = "";
	S.setSessionTokens({ input: 0, output: 0 });
	updateTokenBar();
}

var chatHandlers = {
	thinking: handleChatThinking,
	thinking_text: handleChatThinkingText,
	thinking_done: handleChatThinkingDone,
	voice_pending: handleChatVoicePending,
	tool_call_start: handleChatToolCallStart,
	tool_call_end: handleChatToolCallEnd,
	channel_user: handleChatChannelUser,
	delta: handleChatDelta,
	final: handleChatFinal,
	auto_compact: handleChatAutoCompact,
	error: handleChatError,
	notice: handleChatNotice,
	queue_cleared: handleChatQueueCleared,
	session_cleared: handleChatSessionCleared,
};

function handleChatEvent(p) {
	var eventSession = p.sessionKey || sessionStore.activeSessionKey.value;
	var isActive = eventSession === sessionStore.activeSessionKey.value;
	var isChatPage = currentPrefix === "/chats";

	if (isActive && sessionStore.switchInProgress.value) return;

	if (p.sessionKey && !sessionStore.getByKey(p.sessionKey)) {
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

function handleSandboxHostProvision(payload) {
	var isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (payload.phase === "start") {
		var msg = `Installing ${payload.count || ""} package${payload.count === 1 ? "" : "s"} on host\u2026`;
		chatAddMsg("system", msg);
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		var parts = [];
		if (payload.installed > 0) parts.push(`${payload.installed} installed`);
		if (payload.skipped > 0) parts.push(`${payload.skipped} already present`);
		chatAddMsg("system", `Host packages ready (${parts.join(", ") || "done"})`);
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Host package install failed: ${payload.error || "unknown"}`);
	}
}

function handleBrowserImagePull(payload) {
	var isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	var image = payload.image || "browser container";
	if (payload.phase === "start") {
		chatAddMsg("system", `Pulling browser container image (${image})\u2026 This may take a few minutes on first run.`);
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("system", `Browser container image ready: ${image}`);
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Browser container image pull failed: ${payload.error || "unknown"}`);
	}
}

// Track download indicator element
var downloadIndicatorEl = null;

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Download progress UI with multiple states
function handleLocalLlmDownload(payload) {
	var isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;

	var modelName = payload.displayName || payload.modelId || "model";

	if (payload.error) {
		// Download error
		if (downloadIndicatorEl) {
			downloadIndicatorEl.remove();
			downloadIndicatorEl = null;
		}
		chatAddMsg("error", `Failed to download ${modelName}: ${payload.error}`);
		return;
	}

	if (payload.complete) {
		// Download complete
		if (downloadIndicatorEl) {
			downloadIndicatorEl.remove();
			downloadIndicatorEl = null;
		}
		chatAddMsg("system", `${modelName} ready`);
		return;
	}

	// Download in progress - show/update progress indicator
	if (!downloadIndicatorEl) {
		downloadIndicatorEl = document.createElement("div");
		downloadIndicatorEl.className = "msg system download-indicator";

		var status = document.createElement("div");
		status.className = "download-status";
		status.textContent = `Downloading ${modelName}\u2026`;
		downloadIndicatorEl.appendChild(status);

		var progressContainer = document.createElement("div");
		progressContainer.className = "download-progress";
		var progressBar = document.createElement("div");
		progressBar.className = "download-progress-bar";
		progressContainer.appendChild(progressBar);
		downloadIndicatorEl.appendChild(progressContainer);

		var progressText = document.createElement("div");
		progressText.className = "download-progress-text";
		downloadIndicatorEl.appendChild(progressText);

		if (S.chatMsgBox) {
			S.chatMsgBox.appendChild(downloadIndicatorEl);
			S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
		}
	}

	// Update progress bar
	var barEl = downloadIndicatorEl.querySelector(".download-progress-bar");
	var textEl = downloadIndicatorEl.querySelector(".download-progress-text");
	var containerEl = downloadIndicatorEl.querySelector(".download-progress");

	if (barEl && containerEl) {
		if (payload.progress != null) {
			// Determinate progress - show actual percentage
			containerEl.classList.remove("indeterminate");
			barEl.style.width = `${payload.progress.toFixed(1)}%`;
		} else if (payload.total == null && payload.downloaded != null) {
			// Indeterminate progress - CSS handles the animation
			containerEl.classList.add("indeterminate");
			barEl.style.width = ""; // Let CSS control width
		}
	}

	if (payload.downloaded != null && textEl) {
		var downloadedMb = (payload.downloaded / (1024 * 1024)).toFixed(1);
		if (payload.total != null) {
			var totalMb = (payload.total / (1024 * 1024)).toFixed(1);
			textEl.textContent = `${downloadedMb} / ${totalMb} MB`;
		} else {
			textEl.textContent = `${downloadedMb} MB`;
		}
	}
}

var modelsUpdatedTimer = null;
function handleModelsUpdated(payload) {
	// Progress/status frames are consumed directly by the Providers page.
	// Avoid spamming model refresh requests while a probe is running.
	if (payload?.phase === "start" || payload?.phase === "progress") return;
	if (modelsUpdatedTimer) return;
	modelsUpdatedTimer = setTimeout(() => {
		modelsUpdatedTimer = null;
		// fetchModels() delegates to modelStore.fetch() internally
		fetchModels();
		if (S.refreshProvidersPage) S.refreshProvidersPage();
	}, 150);
}

// ── Location request handler ─────────────────────────────────

function handleWsError(payload) {
	var isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	chatAddErrorMsg(payload.message || "Unknown error");
}

function handleLocationRequest(payload) {
	var requestId = payload.requestId;
	if (!requestId) return;

	if (!navigator.geolocation) {
		sendRpc("location.result", {
			requestId,
			error: { code: 0, message: "Geolocation not supported" },
		});
		return;
	}

	// Coarse: city-level, fast, longer cache. Precise: GPS-level, fresh.
	var coarse = payload.precision === "coarse";
	var geoOpts = coarse
		? { enableHighAccuracy: false, timeout: 10000, maximumAge: 1800000 }
		: { enableHighAccuracy: true, timeout: 15000, maximumAge: 60000 };

	navigator.geolocation.getCurrentPosition(
		(pos) => {
			sendRpc("location.result", {
				requestId,
				location: {
					latitude: pos.coords.latitude,
					longitude: pos.coords.longitude,
					accuracy: pos.coords.accuracy,
				},
			});
		},
		(err) => {
			sendRpc("location.result", {
				requestId,
				error: { code: err.code, message: err.message },
			});
		},
		geoOpts,
	);
}

var eventHandlers = {
	chat: handleChatEvent,
	error: handleWsError,
	"exec.approval.requested": handleApprovalEvent,
	"logs.entry": handleLogEntry,
	"sandbox.image.build": handleSandboxImageBuild,
	"sandbox.image.provision": handleSandboxImageProvision,
	"sandbox.host.provision": handleSandboxHostProvision,
	"browser.image.pull": handleBrowserImagePull,
	"local-llm.download": handleLocalLlmDownload,
	"models.updated": handleModelsUpdated,
	"location.request": handleLocationRequest,
};

function dispatchFrame(frame) {
	if (frame.type !== "event") return;
	var listeners = eventListeners[frame.event] || [];
	listeners.forEach((h) => {
		h(frame.payload || {});
	});
	var handler = eventHandlers[frame.event];
	if (handler) handler(frame.payload || {});
}

var connectOpts = {
	onFrame: dispatchFrame,
	onConnected: (hello) => {
		setStatus("connected", "");
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
	},
	onHandshakeFailed: (frame) => {
		setStatus("", "handshake failed");
		var reason = frame.error?.message || "unknown error";
		chatAddMsg("error", `Handshake failed: ${reason}`);
	},
	onDisconnected: (wasConnected) => {
		if (wasConnected) {
			setStatus("", "disconnected \u2014 reconnecting\u2026");
		}
		// Reset active session's stream state
		var activeS = sessionStore.activeSession.value;
		if (activeS) activeS.resetStreamState();
		S.setStreamEl(null);
		S.setStreamText("");
	},
};

export function connect() {
	setStatus("connecting", "connecting...");
	connectWs(connectOpts);
}

function setStatus(state, text) {
	var dot = S.$("statusDot");
	var sText = S.$("statusText");
	dot.className = `status-dot ${state}`;
	sText.textContent = text;
	sText.classList.toggle("status-text-live", state === "connected");
	var sendBtn = S.$("sendBtn");
	if (sendBtn) sendBtn.disabled = state !== "connected";
}

document.addEventListener("visibilitychange", () => {
	if (!(document.hidden || S.connected)) {
		forceReconnect(connectOpts);
	}
});
