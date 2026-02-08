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
import { formatTokens, renderMarkdown, sendRpc } from "./helpers.js";
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
	var wrap = document.createElement("div");
	wrap.className = "mt-2";
	var audio = document.createElement("audio");
	audio.controls = true;
	audio.preload = "none";
	audio.src = src;
	audio.className = "w-full max-w-md";
	wrap.appendChild(audio);
	msgEl.appendChild(wrap);
	audio.play().catch(() => undefined);
	return true;
}

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

function handleChatVoicePending(_p, isActive, isChatPage) {
	if (!(isActive && isChatPage)) return;
	S.setVoicePending(true);
	removeThinking();
	var el = document.createElement("div");
	el.className = "msg assistant thinking";
	el.id = "voiceGeneratingIndicator";
	var tpl = document.getElementById("tpl-voice-generating");
	el.appendChild(tpl.content.cloneNode(true));
	S.chatMsgBox.appendChild(el);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

/** Build a short summary string for a tool call card. */
function toolCallSummary(name, args, executionMode) {
	if (!args) return name || "tool";
	switch (name) {
		case "exec":
			return args.command || "exec";
		case "web_fetch":
			return `web_fetch ${args.url || ""}`.trim();
		case "web_search":
			return `web_search "${args.query || ""}"`;
		case "browser": {
			// Format: browser action (mode) url
			var action = args.action || "browser";
			var mode = executionMode ? ` (${executionMode})` : "";
			var url = args.url ? ` ${args.url}` : "";
			return `browser ${action}${mode}${url}`.trim();
		}
		default:
			return name || "tool";
	}
}

function handleChatToolCallStart(p, isActive, isChatPage) {
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
	// Browser screenshot support - display as thumbnail with lightbox and download
	if (result.screenshot) {
		var imgContainer = document.createElement("div");
		imgContainer.className = "screenshot-container";
		var img = document.createElement("img");
		// Handle both raw base64 and data URI formats
		var imgSrc = result.screenshot.startsWith("data:")
			? result.screenshot
			: `data:image/png;base64,${result.screenshot}`;
		img.src = imgSrc;
		img.className = "screenshot-thumbnail";
		img.alt = "Browser screenshot";
		img.title = "Click to view full size";

		// Scale factor for HiDPI/Retina displays (default to 1 if not provided)
		var scale = result.screenshot_scale || 1;

		// Once image loads, set display size based on scale factor
		// This makes 2x screenshots display at their logical size (crisp on Retina)
		img.onload = () => {
			if (scale > 1) {
				var logicalWidth = img.naturalWidth / scale;
				var logicalHeight = img.naturalHeight / scale;
				// Cap thumbnail at max-width from CSS, but set aspect ratio
				img.style.aspectRatio = `${logicalWidth} / ${logicalHeight}`;
			}
		};

		// Helper to trigger download
		var downloadScreenshot = (e) => {
			e.stopPropagation();
			var link = document.createElement("a");
			link.href = imgSrc;
			link.download = `screenshot-${Date.now()}.png`;
			link.click();
		};

		img.onclick = () => {
			// Create fullscreen lightbox overlay
			var overlay = document.createElement("div");
			overlay.className = "screenshot-lightbox";

			// Container for image and controls
			var lightboxContent = document.createElement("div");
			lightboxContent.className = "screenshot-lightbox-content";

			// Header with close button and download button
			var header = document.createElement("div");
			header.className = "screenshot-lightbox-header";
			header.onclick = (e) => e.stopPropagation();

			var closeBtn = document.createElement("button");
			closeBtn.className = "screenshot-lightbox-close";
			closeBtn.innerHTML = "✕";
			closeBtn.title = "Close (Esc)";
			closeBtn.onclick = () => overlay.remove();

			var downloadBtn = document.createElement("button");
			downloadBtn.className = "screenshot-download-btn";
			downloadBtn.innerHTML = "⬇ Download";
			downloadBtn.onclick = downloadScreenshot;

			header.appendChild(closeBtn);
			header.appendChild(downloadBtn);

			// Scrollable container for the image
			var scrollContainer = document.createElement("div");
			scrollContainer.className = "screenshot-lightbox-scroll";
			scrollContainer.onclick = (e) => e.stopPropagation();

			var fullImg = document.createElement("img");
			fullImg.src = img.src;
			fullImg.className = "screenshot-lightbox-img";

			// Scale lightbox image for proper display on HiDPI screens
			// For tall screenshots, use a reasonable width to allow vertical scrolling
			fullImg.onload = () => {
				var logicalWidth = fullImg.naturalWidth / scale;
				var logicalHeight = fullImg.naturalHeight / scale;
				var viewportWidth = window.innerWidth - 80; // Account for padding

				// Use logical width, but cap at viewport width minus padding
				var displayWidth = Math.min(logicalWidth, viewportWidth);
				fullImg.style.width = `${displayWidth}px`;

				// Height scales proportionally - will overflow and scroll for tall images
				var displayHeight = (displayWidth / logicalWidth) * logicalHeight;
				fullImg.style.height = `${displayHeight}px`;
			};

			scrollContainer.appendChild(fullImg);
			lightboxContent.appendChild(header);
			lightboxContent.appendChild(scrollContainer);
			overlay.appendChild(lightboxContent);

			// Close on click outside image
			overlay.onclick = () => overlay.remove();
			// Close on Escape key
			var closeOnEscape = (e) => {
				if (e.key === "Escape") {
					overlay.remove();
					document.removeEventListener("keydown", closeOnEscape);
				}
			};
			document.addEventListener("keydown", closeOnEscape);
			document.body.appendChild(overlay);
		};

		// Download button next to thumbnail
		var thumbDownloadBtn = document.createElement("button");
		thumbDownloadBtn.className = "screenshot-download-btn-small";
		thumbDownloadBtn.innerHTML = "⬇";
		thumbDownloadBtn.title = "Download screenshot";
		thumbDownloadBtn.onclick = downloadScreenshot;

		imgContainer.appendChild(img);
		imgContainer.appendChild(thumbDownloadBtn);
		toolCard.appendChild(imgContainer);
	}
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Tool result processing with multiple cases
function handleChatToolCallEnd(p, isActive, isChatPage) {
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
		appendToolResult(toolCard, p.result);
	} else if (!p.success && p.error && p.error.detail) {
		var errMsg = document.createElement("div");
		errMsg.className = isValidationError ? "exec-retry-detail" : "exec-error-detail";
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

function resolveFinalMessageEl(p) {
	// Empty / whitespace-only responses are persisted for LLM history but hidden from the UI.
	if (!(p.text && p.text.trim())) {
		if (S.streamEl) S.streamEl.remove();
		return null;
	}
	var isEcho =
		S.lastToolOutput &&
		p.text.replace(/[`\s]/g, "").indexOf(S.lastToolOutput.replace(/\s/g, "").substring(0, 80)) !== -1;
	if (!isEcho) {
		if (S.streamEl) {
			setSafeMarkdownHtml(S.streamEl, p.text);
			return S.streamEl;
		}
		return chatAddMsg("assistant", renderMarkdown(p.text), true);
	}
	if (S.streamEl) S.streamEl.remove();
	return null;
}

function appendFinalFooter(msgEl, p) {
	// When the LLM says nothing (empty response), fall back to the last
	// visible element (typically the exec card) so the footer is still shown.
	if (!msgEl && p.model && S.chatMsgBox) {
		msgEl = S.chatMsgBox.lastElementChild;
	}
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
	if (p.messageIndex !== undefined && p.messageIndex <= S.lastHistoryIndex) {
		setSessionReplying(eventSession, false);
		return;
	}
	bumpSessionCount(eventSession, 1);
	setSessionReplying(eventSession, false);
	if (!isActive) {
		setSessionUnread(eventSession, true);
	}
	if (!(isActive && isChatPage)) {
		S.setVoicePending(false);
		return;
	}
	removeThinking();
	// Remove voice-generating indicator if present
	var voiceIndicator = document.getElementById("voiceGeneratingIndicator");
	if (voiceIndicator) voiceIndicator.remove();

	if (S.voicePending && p.text && p.replyMedium === "voice") {
		// Voice pending path: we suppressed streaming, so render everything at once.
		var msgEl = S.streamEl || document.createElement("div");
		msgEl.className = "msg assistant";
		msgEl.textContent = "";
		if (!msgEl.parentNode) S.chatMsgBox.appendChild(msgEl);

		if (p.audio) {
			var filename = p.audio.split("/").pop();
			var audioSrc = `/api/sessions/${encodeURIComponent(p.sessionKey || S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			renderAudioPlayer(msgEl, audioSrc, true);
		}
		// Render transcript text below the audio
		var textWrap = document.createElement("div");
		textWrap.className = "mt-2";
		setSafeMarkdownHtml(textWrap, p.text);
		msgEl.appendChild(textWrap);
		appendFinalFooter(msgEl, p);
		S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
	} else {
		var resolvedEl = resolveFinalMessageEl(p);
		if (resolvedEl && p.text && p.replyMedium === "voice") {
			if (p.audio) {
				var fn2 = p.audio.split("/").pop();
				var src2 = `/api/sessions/${encodeURIComponent(p.sessionKey || S.activeSessionKey)}/media/${encodeURIComponent(fn2)}`;
				resolvedEl.textContent = "";
				renderAudioPlayer(resolvedEl, src2, true);
				appendFinalFooter(resolvedEl, p);
			} else {
				appendAssistantVoiceIfEnabled(resolvedEl, p.text)
					.catch((err) => {
						console.warn("Web UI TTS playback failed:", err);
						return false;
					})
					.finally(() => appendFinalFooter(resolvedEl, p));
			}
		} else {
			appendFinalFooter(resolvedEl, p);
		}
	}
	if (p.inputTokens || p.outputTokens) {
		S.sessionTokens.input += p.inputTokens || 0;
		S.sessionTokens.output += p.outputTokens || 0;
		updateTokenBar();
	}
	appendLastMessageTimestamp(Date.now());
	S.setStreamEl(null);
	S.setStreamText("");
	S.setLastToolOutput("");
	S.setVoicePending(false);
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
	if (!(isActive && isChatPage)) {
		S.setVoicePending(false);
		return;
	}
	removeThinking();
	var voiceIndicator = document.getElementById("voiceGeneratingIndicator");
	if (voiceIndicator) voiceIndicator.remove();
	if (p.error?.title) {
		chatAddErrorCard(p.error);
	} else {
		chatAddErrorMsg(p.message || "unknown");
	}
	S.setStreamEl(null);
	S.setStreamText("");
	S.setVoicePending(false);
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
	voice_pending: handleChatVoicePending,
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
		fetchModels();
		if (S.refreshProvidersPage) S.refreshProvidersPage();
	}, 150);
}

var eventHandlers = {
	chat: handleChatEvent,
	"exec.approval.requested": handleApprovalEvent,
	"logs.entry": handleLogEntry,
	"sandbox.image.build": handleSandboxImageBuild,
	"sandbox.image.provision": handleSandboxImageProvision,
	"browser.image.pull": handleBrowserImagePull,
	"local-llm.download": handleLocalLlmDownload,
	"models.updated": handleModelsUpdated,
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
		setStatus("connected", "live");
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
		S.setStreamEl(null);
		S.setStreamText("");
		S.setVoicePending(false);
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
