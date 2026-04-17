// ── Chat page ────────────────────────────────────────────

import { effect } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { chatAddMsg, chatAddMsgWithImages, updateCommandInputUI } from "./chat-ui.js";
import { highlightCodeBlocks } from "./code-highlight.js";
import { SessionHeader } from "./components/session-header.js";
import { formatBytes, formatTokens, renderMarkdown, sendRpc, warmAudioPlayback } from "./helpers.js";
import {
	clearPendingImages,
	getPendingImages,
	hasPendingImages,
	initMediaDrop,
	teardownMediaDrop,
} from "./media-drop.js";
import { bindModelComboEvents, setSessionModel } from "./models.js";
import { bindNodeComboEvents, fetchNodes, unbindNodeEvents } from "./nodes-selector.js";
import { bindReasoningToggle, unbindReasoningToggle } from "./reasoning-toggle.js";
import { registerPrefix, sessionPath } from "./router.js";
import { routes } from "./routes.js";
import { bindSandboxImageEvents, bindSandboxToggleEvents, updateSandboxImageUI, updateSandboxUI } from "./sandbox.js";
import {
	bumpSessionCount,
	cacheOutgoingUserMessage,
	clearActiveSession,
	clearAllSessions,
	seedSessionPreviewFromUserText,
	setSessionActiveRunId,
	setSessionReplying,
	switchSession,
} from "./sessions.js";
import * as S from "./state.js";
import { modelStore } from "./stores/model-store.js";
import { sessionStore } from "./stores/session-store.js";
import { initVoiceInput, teardownVoiceInput } from "./voice-input.js";

// ── Slash commands ───────────────────────────────────────
var slashCommands = [
	{ name: "clear", description: "Clear conversation history" },
	{ name: "compact", description: "Summarize conversation to save tokens" },
	{ name: "context", description: "Show session context and project info" },
	{ name: "sh", description: "Enter command mode (/sh off or Esc to exit)" },
];
var slashMenuEl = null;
var slashMenuIdx = 0;
var slashMenuItems = [];
var chatMoreModalKeydownHandler = null;
var disposeSessionControlsVisibility = null;
var promptMemoryToolbarRequestId = 0;

function slashInjectStyles() {
	if (document.getElementById("slashMenuStyles")) return;
	var s = document.createElement("style");
	s.id = "slashMenuStyles";
	s.textContent =
		".slash-menu{position:absolute;bottom:100%;left:0;right:0;background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-sm);margin-bottom:4px;overflow:hidden;z-index:50;box-shadow:var(--shadow-md);animation:.1s ease-out msg-in}" +
		".slash-menu-item{padding:7px 12px;cursor:pointer;display:flex;align-items:center;gap:8px;font-size:.8rem;color:var(--text);transition:background .1s}" +
		".slash-menu-item:hover,.slash-menu-item.active{background:var(--bg-hover)}" +
		".slash-menu-item .slash-name{font-weight:600;color:var(--accent);font-family:var(--font-mono);font-size:.78rem}" +
		".slash-menu-item .slash-desc{color:var(--muted);font-size:.75rem}" +
		".ctx-card{background:var(--surface);border:1px solid var(--border);border-radius:var(--radius);align-self:center;max-width:520px;width:100%;padding:0;font-size:.8rem;line-height:1.55;animation:.2s ease-out msg-in;overflow:hidden;flex-shrink:0}" +
		".ctx-header{background:var(--surface2);padding:10px 16px;border-bottom:1px solid var(--border);display:flex;align-items:center;gap:8px}" +
		".ctx-header svg,.ctx-header .icon{flex-shrink:0;opacity:.7}" +
		".ctx-header-title{font-weight:600;font-size:.85rem;color:var(--text)}" +
		".ctx-section{padding:10px 16px;border-bottom:1px solid var(--border)}" +
		".ctx-section:last-child{border-bottom:none}" +
		".ctx-section-title{font-weight:600;font-size:.72rem;text-transform:uppercase;letter-spacing:.05em;color:var(--muted);margin-bottom:6px}" +
		".ctx-row{display:flex;gap:8px;padding:2px 0;align-items:baseline}" +
		".ctx-label{color:var(--muted);min-width:80px;flex-shrink:0;font-size:.78rem}" +
		".ctx-value{color:var(--text);word-break:break-all;font-size:.78rem}" +
		".ctx-value.mono{font-family:var(--font-mono);font-size:.74rem}" +
		".ctx-tag{display:inline-flex;align-items:center;gap:4px;background:var(--surface2);border:1px solid var(--border);border-radius:var(--radius-sm);padding:2px 8px;font-size:.72rem;color:var(--text);margin:2px 2px 2px 0}" +
		".ctx-tag .ctx-tag-dot{width:6px;height:6px;border-radius:50%;background:var(--accent);flex-shrink:0}" +
		".ctx-file{font-family:var(--font-mono);font-size:.72rem;color:var(--muted);padding:3px 0;display:flex;justify-content:space-between;gap:12px}" +
		".ctx-file-path{color:var(--text);word-break:break-all}" +
		".ctx-file-size{flex-shrink:0;opacity:.7}" +
		".ctx-empty{color:var(--muted);font-style:italic;font-size:.78rem;padding:2px 0}" +
		".ctx-warning{background:var(--warning-bg,rgba(234,179,8,.15));border:1px solid var(--warning-border,rgba(234,179,8,.3));border-radius:var(--radius-sm);padding:8px 12px;margin:8px 12px;font-size:.78rem;color:var(--text);display:flex;align-items:center;gap:8px}" +
		".ctx-warning svg,.ctx-warning .icon{flex-shrink:0;color:var(--warning,#eab308)}" +
		".ctx-disabled{color:var(--muted);font-style:italic;font-size:.78rem;padding:2px 0;background:var(--warning-bg,rgba(234,179,8,.1));border-radius:var(--radius-sm);padding:6px 10px;border-left:3px solid var(--warning,#eab308)}";
	document.head.appendChild(s);
}

function slashShowMenu(filter) {
	slashInjectStyles();
	var matches = slashCommands.filter((c) => `/${c.name}`.indexOf(filter) === 0);
	if (matches.length === 0) {
		slashHideMenu();
		return;
	}
	slashMenuItems = matches;
	slashMenuIdx = 0;

	if (!slashMenuEl) {
		slashMenuEl = document.createElement("div");
		slashMenuEl.className = "slash-menu";
	}
	while (slashMenuEl.firstChild) slashMenuEl.removeChild(slashMenuEl.firstChild);
	matches.forEach((cmd, i) => {
		var item = document.createElement("div");
		item.className = `slash-menu-item${i === 0 ? " active" : ""}`;
		var nameSpan = document.createElement("span");
		nameSpan.className = "slash-name";
		nameSpan.textContent = `/${cmd.name}`;
		var descSpan = document.createElement("span");
		descSpan.className = "slash-desc";
		descSpan.textContent = cmd.description;
		item.appendChild(nameSpan);
		item.appendChild(descSpan);
		item.addEventListener("mousedown", (e) => {
			e.preventDefault();
			slashSelectItem(i);
		});
		slashMenuEl.appendChild(item);
	});

	var inputWrap = S.chatInput.parentElement;
	if (inputWrap && !slashMenuEl.parentElement) {
		inputWrap.classList.add("relative");
		inputWrap.appendChild(slashMenuEl);
	}
}

function slashHideMenu() {
	if (slashMenuEl?.parentElement) {
		slashMenuEl.parentElement.removeChild(slashMenuEl);
	}
	slashMenuItems = [];
	slashMenuIdx = 0;
}

function slashSelectItem(idx) {
	if (!slashMenuItems[idx]) return;
	S.chatInput.value = `/${slashMenuItems[idx].name}`;
	slashHideMenu();
	sendChat();
}

function slashHandleInput() {
	var val = S.chatInput.value;
	if (val.indexOf("/") === 0 && val.indexOf(" ") === -1) {
		slashShowMenu(val);
	} else {
		slashHideMenu();
	}
}

function slashHandleKeydown(e) {
	if (!slashMenuEl?.parentElement || slashMenuItems.length === 0) return false;
	if (e.key === "ArrowUp") {
		e.preventDefault();
		slashMenuIdx = (slashMenuIdx - 1 + slashMenuItems.length) % slashMenuItems.length;
		slashUpdateActive();
		return true;
	}
	if (e.key === "ArrowDown") {
		e.preventDefault();
		slashMenuIdx = (slashMenuIdx + 1) % slashMenuItems.length;
		slashUpdateActive();
		return true;
	}
	if (e.key === "Enter" || e.key === "Tab") {
		e.preventDefault();
		slashSelectItem(slashMenuIdx);
		return true;
	}
	if (e.key === "Escape") {
		e.preventDefault();
		slashHideMenu();
		return true;
	}
	return false;
}

function slashUpdateActive() {
	if (!slashMenuEl) return;
	var items = slashMenuEl.querySelectorAll(".slash-menu-item");
	items.forEach((el, i) => {
		el.classList.toggle("active", i === slashMenuIdx);
	});
}

function parseSlashCommand(text) {
	if (!text || text.charAt(0) !== "/") return null;
	var body = text.substring(1).trim();
	if (!body) return null;
	var spaceIdx = body.indexOf(" ");
	if (spaceIdx === -1) {
		return { name: body.toLowerCase(), args: "" };
	}
	return {
		name: body.substring(0, spaceIdx).toLowerCase(),
		args: body.substring(spaceIdx + 1).trim(),
	};
}

function isShLocalToggle(args) {
	if (!args) return true;
	var normalized = args.toLowerCase();
	return normalized === "on" || normalized === "off" || normalized === "exit";
}

function shouldHandleSlashLocally(cmdName, args) {
	if (cmdName === "sh") {
		return isShLocalToggle(args);
	}
	return slashCommands.some((c) => c.name === cmdName);
}

function commandModeSummary() {
	var execModeLabel = S.sessionExecMode === "sandbox" ? "sandboxed" : "host";
	var promptSymbol = S.sessionExecPromptSymbol || "$";
	return `${execModeLabel}, prompt ${promptSymbol}`;
}

function setCommandMode(enabled) {
	S.setCommandModeEnabled(!!enabled);
	updateCommandInputUI();
}

// ── Context card helpers ─────────────────────────────────
function ctxEl(tag, cls, text) {
	var el = document.createElement(tag);
	if (cls) el.className = cls;
	if (text !== undefined) el.textContent = text;
	return el;
}

function ctxRow(label, value, mono) {
	var row = ctxEl("div", "ctx-row");
	row.appendChild(ctxEl("span", "ctx-label", label));
	row.appendChild(ctxEl("span", `ctx-value${mono ? " mono" : ""}`, value));
	return row;
}

function ctxSection(title) {
	var sec = ctxEl("div", "ctx-section");
	sec.appendChild(ctxEl("div", "ctx-section-title", title));
	return sec;
}

function formatPromptMemoryMode(mode) {
	if (mode === "frozen-at-session-start") return "Frozen at session start";
	if (mode === "live-reload") return "Live reload";
	return mode || "unknown";
}

function formatPromptMemorySource(source) {
	if (source === "agent_workspace") return "Agent workspace";
	if (source === "root_workspace") return "Root workspace";
	return source || "unknown";
}

function buildPromptMemorySummary(promptMemory) {
	if (!promptMemory) return "Unavailable";
	var parts = [formatPromptMemoryMode(promptMemory.mode)];
	if (promptMemory.snapshotActive) {
		parts.push("snapshot active");
	}
	parts.push(promptMemory.present ? `${Number(promptMemory.chars || 0).toLocaleString()} chars` : "empty");
	return parts.join(" · ");
}

function promptMemoryDetailParts(promptMemory) {
	if (!promptMemory) return [];
	var parts = [];
	if (promptMemory.fileSource) parts.push(`source ${formatPromptMemorySource(promptMemory.fileSource)}`);
	if (promptMemory.path) parts.push(promptMemory.path);
	return parts;
}

function promptMemoryToolbarTitle(promptMemory) {
	if (!promptMemory) return "Prompt memory unavailable";
	var parts = [`Prompt memory: ${buildPromptMemorySummary(promptMemory)}`];
	var detailParts = promptMemoryDetailParts(promptMemory);
	if (detailParts.length > 0) parts.push(detailParts.join(" · "));
	return parts.join("\n");
}

function promptMemoryToolbarLabel(promptMemory) {
	if (!promptMemory) return "Memory";
	if (promptMemory.mode === "frozen-at-session-start") return "Memory frozen";
	if (promptMemory.mode === "live-reload") return "Memory live";
	return "Memory";
}

function setPromptMemoryToolbarState(promptMemory, loading, refreshing) {
	var toolbar = S.$("promptMemoryToolbar");
	var statusBtn = S.$("promptMemoryStatusBtn");
	var statusLabel = S.$("promptMemoryStatusLabel");
	var refreshBtn = S.$("promptMemoryRefreshBtn");
	if (!(toolbar && statusBtn && statusLabel && refreshBtn)) return;
	toolbar.classList.remove("hidden");
	toolbar.classList.add("inline-flex");
	statusBtn.disabled = !!loading;
	refreshBtn.disabled = !!refreshing;
	if (loading) {
		statusLabel.textContent = "Memory…";
		statusBtn.title = "Loading prompt memory status";
		refreshBtn.classList.add("hidden");
		return;
	}
	statusLabel.textContent = promptMemoryToolbarLabel(promptMemory);
	statusBtn.title = promptMemoryToolbarTitle(promptMemory);
	refreshBtn.classList.toggle("hidden", promptMemory?.mode !== "frozen-at-session-start");
	refreshBtn.title =
		promptMemory?.mode === "frozen-at-session-start" ? "Refresh frozen prompt memory" : "Refresh unavailable";
}

function refreshPromptMemoryToolbarFromPayload(promptMemory) {
	setPromptMemoryToolbarState(promptMemory || null, false, false);
}

function refreshPromptMemoryToolbar() {
	if (!S.connected) {
		setPromptMemoryToolbarState(null, false, false);
		return Promise.resolve(null);
	}
	var requestId = ++promptMemoryToolbarRequestId;
	setPromptMemoryToolbarState(null, true, false);
	return sendRpc("chat.context", {}).then((res) => {
		if (requestId !== promptMemoryToolbarRequestId) return null;
		if (res?.ok && res.payload) {
			var promptMemory = res.payload.promptMemory || null;
			refreshPromptMemoryToolbarFromPayload(promptMemory);
			return promptMemory;
		}
		setPromptMemoryToolbarState(null, false, false);
		return null;
	});
}

function refreshPromptMemoryToolbarSnapshot() {
	setPromptMemoryToolbarState(null, false, true);
	return sendRpc("chat.prompt_memory.refresh", {})
		.then((res) => {
			if (!(res?.ok && res.payload)) {
				throw new Error(res?.error?.message || "Failed to refresh prompt memory");
			}
			var promptMemory = res.payload.promptMemory || null;
			refreshPromptMemoryToolbarFromPayload(promptMemory);
			maybeRefreshFullContext();
			return promptMemory;
		})
		.catch((error) => {
			refreshPromptMemoryToolbar();
			chatAddMsg("error", error?.message || "Failed to refresh prompt memory");
			return null;
		});
}

// ── Context card per-section renderers ───────────────────
function renderContextSessionSection(card, data) {
	var sess = data.session || {};
	var sessSection = ctxSection("Session");
	sessSection.appendChild(ctxRow("Key", sess.key || "unknown", true));
	sessSection.appendChild(ctxRow("Messages", String(sess.messageCount || 0)));
	sessSection.appendChild(ctxRow("Model", sess.model || "default", true));
	if (sess.provider) sessSection.appendChild(ctxRow("Provider", sess.provider, true));
	if (sess.label) sessSection.appendChild(ctxRow("Label", sess.label));
	sessSection.appendChild(ctxRow("Tool Support", data.supportsTools === false ? "Disabled" : "Enabled"));
	card.appendChild(sessSection);
}

function renderContextProjectSection(card, data) {
	var proj = data.project;
	var projSection = ctxSection("Project");
	if (proj && proj !== null) {
		projSection.appendChild(ctxRow("Name", proj.label || "(unnamed)"));
		if (proj.directory) projSection.appendChild(ctxRow("Directory", proj.directory, true));
		if (proj.systemPrompt) projSection.appendChild(ctxRow("System Prompt", `${proj.systemPrompt.length} chars`));
		var ctxFiles = proj.contextFiles || [];
		if (ctxFiles.length > 0) {
			var filesLabel = ctxEl("div", "ctx-section-title", `Context Files (${ctxFiles.length})`);
			filesLabel.classList.add("spaced");
			projSection.appendChild(filesLabel);
			ctxFiles.forEach((f) => {
				var row = ctxEl("div", "ctx-file");
				row.appendChild(ctxEl("span", "ctx-file-path", f.path));
				row.appendChild(ctxEl("span", "ctx-file-size", formatBytes(f.size)));
				projSection.appendChild(row);
			});
		}
	} else {
		projSection.appendChild(ctxEl("div", "ctx-empty", "No project bound to this session"));
	}
	card.appendChild(projSection);
}

function renderContextToolsSection(card, data) {
	var tools = data.tools || [];
	var toolsSection = ctxSection("Tools");
	if (data.supportsTools === false) {
		toolsSection.appendChild(ctxEl("div", "ctx-disabled", "Tools disabled \u2014 model doesn't support tool calling"));
	} else if (tools.length > 0) {
		var toolWrap = ctxEl("div", "");
		toolWrap.className = "ctx-tool-wrap";
		tools.forEach((t) => {
			var tag = ctxEl("span", "ctx-tag");
			var dot = ctxEl("span", "ctx-tag-dot");
			tag.appendChild(dot);
			tag.appendChild(document.createTextNode(t.name));
			tag.title = t.description;
			toolWrap.appendChild(tag);
		});
		toolsSection.appendChild(toolWrap);
	} else {
		toolsSection.appendChild(ctxEl("div", "ctx-empty", "No tools registered"));
	}
	card.appendChild(toolsSection);
}

function renderContextSkillsSection(card, data) {
	var skills = data.skills || [];
	var skillsSection = ctxSection("Skills & Plugins");
	if (data.supportsTools === false) {
		skillsSection.appendChild(
			ctxEl("div", "ctx-disabled", "Skills disabled \u2014 model doesn't support tool calling"),
		);
	} else if (skills.length > 0) {
		var wrap = ctxEl("div", "");
		wrap.className = "ctx-tool-wrap";
		skills.forEach((s) => {
			var tag = ctxEl("span", "ctx-tag");
			var dot = ctxEl("span", "ctx-tag-dot");
			var isPlugin = s.source === "plugin";
			dot.style.background = isPlugin ? "var(--accent)" : "var(--success, #4a9)";
			tag.appendChild(dot);
			tag.appendChild(document.createTextNode(s.name));
			tag.title = (isPlugin ? "[Plugin] " : "[Skill] ") + (s.description || "");
			wrap.appendChild(tag);
		});
		skillsSection.appendChild(wrap);
	} else {
		skillsSection.appendChild(ctxEl("div", "ctx-empty", "No skills or plugins enabled"));
	}
	card.appendChild(skillsSection);
}

function renderContextMcpSection(card, data) {
	var servers = data.mcpServers || [];
	var section = ctxSection("MCP Tools");
	if (data.supportsTools === false) {
		section.appendChild(ctxEl("div", "ctx-disabled", "MCP tools disabled \u2014 model doesn't support tool calling"));
	} else if (data.mcpDisabled) {
		section.appendChild(ctxEl("div", "ctx-disabled", "MCP tools disabled for this session"));
	} else {
		var running = servers.filter((s) => s.state === "running");
		if (running.length > 0) {
			var wrap = ctxEl("div", "");
			wrap.className = "ctx-tool-wrap";
			running.forEach((s) => {
				var tag = ctxEl("span", "ctx-tag");
				var dot = ctxEl("span", "ctx-tag-dot");
				dot.style.background = "var(--ok)";
				tag.appendChild(dot);
				tag.appendChild(document.createTextNode(s.name));
				tag.title = `${s.tool_count} tool${s.tool_count !== 1 ? "s" : ""} — ${s.state}`;
				wrap.appendChild(tag);
			});
			section.appendChild(wrap);
		} else {
			section.appendChild(ctxEl("div", "ctx-empty", "No MCP tools running"));
		}
	}
	card.appendChild(section);
}

function renderContextSandboxSection(card, data) {
	var sb = data.sandbox || {};
	var exec = data.execution || {};
	var sandboxSection = ctxSection("Sandbox");
	sandboxSection.appendChild(ctxRow("Enabled", sb.enabled ? "yes" : "no", true));
	var execLabel = exec.mode ? (exec.mode === "sandbox" ? "sandboxed" : "host") : "";
	if (execLabel && exec.promptSymbol) execLabel += ` (${exec.promptSymbol})`;
	if (execLabel) sandboxSection.appendChild(ctxRow("Command route", execLabel, true));
	for (var [label, value, mono] of [
		["Backend", sb.backend, false],
		["Mode", sb.mode, false],
		["Scope", sb.scope, false],
		["Workspace Mount", sb.workspaceMount, false],
		["Image", sb.image, true],
		["Container", sb.containerName, false],
	]) {
		if (value) sandboxSection.appendChild(ctxRow(label, value, mono));
	}
	card.appendChild(sandboxSection);
}

function renderContextTokensSection(card, data) {
	var tu = data.tokenUsage || {};
	var sessionInput = tu.inputTokens || 0;
	var sessionOutput = tu.outputTokens || 0;
	var sessionCacheRead = tu.cacheReadTokens || 0;
	var sessionCacheWrite = tu.cacheWriteTokens || 0;
	var sessionTotal = tu.total || 0;
	var currentInput = tu.currentInputTokens || sessionInput;
	var currentOutput = tu.currentOutputTokens || 0;
	var currentCacheRead = tu.currentCacheReadTokens || 0;
	var currentCacheWrite = tu.currentCacheWriteTokens || 0;
	var currentTotal = tu.currentTotal || currentInput + currentOutput;
	var estimatedNextInput = tu.estimatedNextInputTokens || currentInput;
	var tokenSection = ctxSection("Token Usage");
	tokenSection.appendChild(ctxRow("Session input", formatTokens(sessionInput), true));
	tokenSection.appendChild(ctxRow("Session output", formatTokens(sessionOutput), true));
	if (sessionCacheRead > 0) {
		tokenSection.appendChild(ctxRow("Session cached input", formatTokens(sessionCacheRead), true));
	}
	if (sessionCacheWrite > 0) {
		tokenSection.appendChild(ctxRow("Session cache writes", formatTokens(sessionCacheWrite), true));
	}
	tokenSection.appendChild(ctxRow("Session total", formatTokens(sessionTotal), true));
	tokenSection.appendChild(ctxRow("Current input", formatTokens(currentInput), true));
	tokenSection.appendChild(ctxRow("Current output", formatTokens(currentOutput), true));
	if (currentCacheRead > 0) {
		tokenSection.appendChild(ctxRow("Current cached input", formatTokens(currentCacheRead), true));
	}
	if (currentCacheWrite > 0) {
		tokenSection.appendChild(ctxRow("Current cache writes", formatTokens(currentCacheWrite), true));
	}
	tokenSection.appendChild(ctxRow("Current total", formatTokens(currentTotal), true));
	tokenSection.appendChild(ctxRow("Estimated next input", formatTokens(estimatedNextInput), true));
	if (tu.contextWindow > 0) {
		var pct = Math.max(0, 100 - Math.round((estimatedNextInput / tu.contextWindow) * 100));
		tokenSection.appendChild(ctxRow("Context left", `${pct}% of ${formatTokens(tu.contextWindow)}`, true));
	}
	card.appendChild(tokenSection);
}

function renderContextPromptMemorySection(card, data) {
	var promptMemory = data.promptMemory || null;
	var section = ctxSection("Prompt Memory");
	section.appendChild(ctxRow("Status", buildPromptMemorySummary(promptMemory)));
	if (promptMemory) {
		section.appendChild(ctxRow("Mode", formatPromptMemoryMode(promptMemory.mode)));
		section.appendChild(ctxRow("Present", promptMemory.present ? "yes" : "no"));
		section.appendChild(ctxRow("Chars", Number(promptMemory.chars || 0).toLocaleString(), true));
		if (promptMemory.fileSource) {
			section.appendChild(ctxRow("Source", formatPromptMemorySource(promptMemory.fileSource)));
		}
		if (promptMemory.path) {
			section.appendChild(ctxRow("Path", promptMemory.path, true));
		}
	}
	card.appendChild(section);
}

function renderContextCard(data) {
	if (!S.chatMsgBox) return;
	slashInjectStyles();

	var card = ctxEl("div", "ctx-card");

	var header = ctxEl("div", "ctx-header");
	var icon = document.createElement("span");
	icon.className = "icon icon-settings-gear";
	header.appendChild(icon);
	header.appendChild(ctxEl("span", "ctx-header-title", "Context"));
	card.appendChild(header);

	// Show warning if tools are disabled
	if (data.supportsTools === false) {
		var warning = ctxEl("div", "ctx-warning");
		var warnIcon = document.createElement("span");
		warnIcon.className = "icon icon-warn-triangle-light";
		warning.appendChild(warnIcon);
		warning.appendChild(
			document.createTextNode(
				"Tools disabled \u2014 the current model doesn't support tool calling. Running in chat-only mode.",
			),
		);
		card.appendChild(warning);
	}

	renderContextSessionSection(card, data);
	renderContextProjectSection(card, data);
	renderContextSkillsSection(card, data);
	renderContextMcpSection(card, data);
	renderContextToolsSection(card, data);
	renderContextSandboxSection(card, data);
	renderContextPromptMemorySection(card, data);
	renderContextTokensSection(card, data);

	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

// Human-readable labels for each CompactionMode variant. Keep the keys in
// sync with `compaction_run::compaction_mode_key` in the Rust backend —
// those snake_case keys are what ship in the `mode` field of the
// auto_compact / compact broadcast events.
var COMPACTION_MODE_LABELS = {
	deterministic: "Deterministic",
	recency_preserving: "Recency preserving",
	structured: "Structured",
	llm_replace: "LLM replace",
};

function compactionModeLabel(mode) {
	if (!mode) return "Unknown";
	return COMPACTION_MODE_LABELS[mode] || mode;
}

export function renderCompactCard(data) {
	if (!S.chatMsgBox) return;
	slashInjectStyles();

	var card = ctxEl("div", "ctx-card");

	var header = ctxEl("div", "ctx-header");
	var icon = document.createElement("span");
	icon.className = "icon icon-compress";
	header.appendChild(icon);
	header.appendChild(ctxEl("span", "ctx-header-title", "Conversation compacted"));
	card.appendChild(header);

	// Strategy section — surfaces the mode that actually ran and the LLM
	// token spend so users can see why compaction was fast/slow and what
	// it cost. Fields come from CompactionOutcome::broadcast_metadata()
	// on the Rust side.
	if (data.mode) {
		var strategySection = ctxSection("Strategy");
		strategySection.appendChild(ctxRow("Mode", compactionModeLabel(data.mode)));
		var totalTokens = Number(data.compactionTotalTokens || 0);
		if (totalTokens > 0) {
			var inputTokens = Number(data.compactionInputTokens || 0);
			var outputTokens = Number(data.compactionOutputTokens || 0);
			strategySection.appendChild(
				ctxRow(
					"Tokens used",
					`${formatTokens(totalTokens)} (${formatTokens(inputTokens)} in + ${formatTokens(outputTokens)} out)`,
				),
			);
		} else {
			strategySection.appendChild(ctxRow("Tokens used", "0 (no LLM call)"));
		}
		card.appendChild(strategySection);
	}

	var statsSection = ctxSection("Before compact");
	statsSection.appendChild(ctxRow("Messages", String(data.messageCount || 0)));
	if (data.totalTokens) {
		statsSection.appendChild(ctxRow("Total tokens", formatTokens(data.totalTokens || 0)));
	}
	if (data.estimatedNextInputTokens) {
		statsSection.appendChild(ctxRow("Estimated next input", formatTokens(data.estimatedNextInputTokens), true));
	}
	if (data.contextWindow) {
		var basis = data.estimatedNextInputTokens || data.totalTokens || 0;
		var pctUsed = Math.round((basis / data.contextWindow) * 100);
		statsSection.appendChild(ctxRow("Context usage", `${pctUsed}% of ${formatTokens(data.contextWindow)}`));
	}
	card.appendChild(statsSection);

	// "After compact" messaging depends on the mode: deterministic /
	// llm_replace collapse to a single summary message, while
	// recency_preserving / structured splice the summary between head
	// and tail so multiple messages survive.
	var afterSection = ctxSection("After compact");
	var replacesAll = data.mode === "deterministic" || data.mode === "llm_replace" || !data.mode;
	if (replacesAll) {
		afterSection.appendChild(ctxRow("Messages", "1 (summary)"));
		afterSection.appendChild(ctxRow("Status", "Conversation history replaced with a summary"));
	} else {
		afterSection.appendChild(ctxRow("Status", "Head + tail preserved verbatim; middle summarised"));
	}
	card.appendChild(afterSection);

	// Settings hint — only rendered when the backend supplies one, so
	// an older backend or a mode that doesn't emit it doesn't leave an
	// empty row behind. The hint spans the full card width (not the
	// two-column label/value ctx-row layout) so it reads like a
	// tooltip/footnote.
	if (data.settingsHint) {
		var hintSection = ctxSection("Configure");
		var hintRow = ctxEl("div", "ctx-value");
		hintRow.textContent = data.settingsHint;
		hintSection.appendChild(hintRow);
		card.appendChild(hintSection);
	}

	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

// ── Debug panel ──────────────────────────────────────────
function setDebugModalOpen(open) {
	var modal = S.$("debugModal");
	if (!modal) return;
	modal.classList.toggle("hidden", !open);
	var btn = S.$("debugPanelBtn");
	if (btn) btn.style.color = open ? "var(--accent)" : "var(--muted)";
}

function setFullContextModalOpen(open) {
	var modal = S.$("fullContextModal");
	if (!modal) return;
	modal.classList.toggle("hidden", !open);
	var btn = S.$("fullContextBtn");
	if (btn) btn.style.color = open ? "var(--accent)" : "var(--muted)";
}

function refreshDebugPanel() {
	var panel = S.$("debugPanel");
	if (!panel) return;
	panel.textContent = "";

	var loading = ctxEl("div", "text-xs text-[var(--muted)]", "Loading context\u2026");
	panel.appendChild(loading);

	sendRpc("chat.context", {}).then((res) => {
		panel.textContent = "";
		if (!(res?.ok && res.payload)) {
			panel.appendChild(ctxEl("div", "text-xs text-[var(--error)]", "Failed to load context"));
			return;
		}
		slashInjectStyles();
		renderContextSessionSection(panel, res.payload);
		renderContextProjectSection(panel, res.payload);
		renderContextSkillsSection(panel, res.payload);
		renderContextMcpSection(panel, res.payload);
		renderContextToolsSection(panel, res.payload);
		renderContextSandboxSection(panel, res.payload);
		renderContextPromptMemorySection(panel, res.payload);
		renderContextTokensSection(panel, res.payload);
		refreshPromptMemoryToolbarFromPayload(res.payload.promptMemory || null);
	});
}

function toggleDebugPanel() {
	var modal = S.$("debugModal");
	if (!modal) return;
	var opening = modal.classList.contains("hidden");
	if (!opening) {
		setDebugModalOpen(false);
		return;
	}
	setFullContextModalOpen(false);
	setDebugModalOpen(true);
	refreshDebugPanel();
}

// ── Full context panel ───────────────────────────────────

var ROLE_COLORS = {
	system: "var(--accent)",
	user: "var(--ok, #22c55e)",
	assistant: "var(--info, #3b82f6)",
	tool: "var(--muted)",
};

function ctxMsgBadge(role) {
	var color = ROLE_COLORS[role] || "var(--text)";
	var badge = ctxEl("span", "text-xs font-semibold uppercase px-1.5 py-0.5 rounded");
	badge.style.cssText = `color:${color};background:color-mix(in srgb, ${color} 15%, transparent)`;
	badge.textContent = role;
	return badge;
}

function ctxMsgMeta(msg, contentStr) {
	var parts = [];
	var chars = contentStr ? contentStr.length : 0;
	if (chars > 0) parts.push(`${chars.toLocaleString()} chars`);
	var toolCalls = msg.tool_calls || [];
	if (toolCalls.length > 0) {
		parts.push(`${toolCalls.length} tool call${toolCalls.length > 1 ? "s" : ""}`);
	}
	if (msg.role === "tool" && msg.tool_call_id) {
		parts.push(`id: ${msg.tool_call_id}`);
	}
	return parts.join(" \xb7 ");
}

function ctxMsgToolCall(tc) {
	var div = ctxEl("div", "mt-1 border border-[var(--border)] rounded-md p-2 bg-[var(--surface)]");
	var hdr = ctxEl("div", "text-xs font-semibold text-[var(--text)] mb-1");
	hdr.textContent = `\ud83d\udee0 ${tc.function?.name || "unknown"}`;
	if (tc.id) {
		hdr.appendChild(ctxEl("span", "font-normal text-[var(--muted)] ml-2", `id: ${tc.id}`));
	}
	div.appendChild(hdr);
	if (tc.function?.arguments) {
		var pre = ctxEl("pre", "text-xs font-mono whitespace-pre-wrap break-words text-[var(--text)]");
		try {
			pre.textContent = JSON.stringify(JSON.parse(tc.function.arguments), null, 2);
		} catch {
			pre.textContent = tc.function.arguments;
		}
		div.appendChild(pre);
	}
	return div;
}

function renderContextMessage(msg, index) {
	var wrapper = ctxEl("div", "mb-2");
	var contentStr = typeof msg.content === "string" ? msg.content : JSON.stringify(msg.content, null, 2);

	// Header row: role badge + index + meta + chevron
	var hdr = ctxEl("div", "flex items-center gap-2 cursor-pointer select-none");
	hdr.appendChild(ctxMsgBadge(msg.role || "unknown"));
	hdr.appendChild(ctxEl("span", "text-xs text-[var(--muted)]", `#${index}`));
	var meta = ctxMsgMeta(msg, contentStr);
	if (meta) hdr.appendChild(ctxEl("span", "text-xs text-[var(--muted)]", meta));
	var chevron = ctxEl("span", "text-xs text-[var(--muted)] ml-auto");
	var startOpen = index !== 0;
	chevron.textContent = startOpen ? "\u25bc" : "\u25b6";
	hdr.appendChild(chevron);
	wrapper.appendChild(hdr);

	// Collapsible body
	var body = ctxEl("div", "mt-1");
	body.style.display = startOpen ? "block" : "none";
	hdr.addEventListener("click", () => {
		var open = body.style.display !== "none";
		body.style.display = open ? "none" : "block";
		chevron.textContent = open ? "\u25b6" : "\u25bc";
	});

	if (contentStr) {
		var pre = ctxEl(
			"pre",
			"text-xs font-mono whitespace-pre-wrap break-words bg-[var(--surface)] border border-[var(--border)] rounded-md p-2 text-[var(--text)]",
		);
		pre.textContent = contentStr;
		body.appendChild(pre);
	}
	for (var tc of msg.tool_calls || []) body.appendChild(ctxMsgToolCall(tc));

	wrapper.appendChild(body);
	return wrapper;
}

function buildFullContextPromptMemoryBox(promptMemory) {
	if (!promptMemory) return null;
	var box = ctxEl(
		"div",
		"text-xs mb-3 rounded-md border border-[var(--border)] bg-[var(--surface)] p-2 text-[var(--text)]",
	);
	var summaryLine = ctxEl("div", "font-semibold");
	summaryLine.textContent = `Prompt memory: ${buildPromptMemorySummary(promptMemory)}`;
	box.appendChild(summaryLine);
	var detailParts = promptMemoryDetailParts(promptMemory);
	if (detailParts.length > 0) {
		box.appendChild(ctxEl("div", "mt-1 text-[var(--muted)]", detailParts.join(" · ")));
	}
	return box;
}

function appendFullContextWorkspaceWarnings(panel, payload) {
	var workspaceFiles = Array.isArray(payload.workspaceFiles) ? payload.workspaceFiles : [];
	if (!payload.truncated || workspaceFiles.length === 0) return;
	var truncatedFiles = workspaceFiles.filter((file) => file?.truncated);
	if (truncatedFiles.length === 0) return;
	var warning = ctxEl(
		"div",
		"text-xs mb-3 rounded-md border border-[var(--border)] bg-[var(--surface)] p-2 text-[var(--text)]",
	);
	warning.textContent = truncatedFiles
		.map((file) => {
			var name = typeof file.name === "string" ? file.name : "workspace file";
			var charCount = Number(file.original_chars || 0).toLocaleString();
			var limitChars = Number(file.limit_chars || 0).toLocaleString();
			var truncatedChars = Number(file.truncated_chars || 0).toLocaleString();
			return `${name}: ${charCount} chars, limit ${limitChars}, truncated by ${truncatedChars}`;
		})
		.join(" | ");
	panel.appendChild(warning);
}

function buildFullContextHeaderRow(payload, onRefresh) {
	var headerRow = ctxEl("div", "flex items-center gap-3 mb-3");
	var headerText = ctxEl("span", "text-xs text-[var(--muted)]");
	headerText.textContent =
		`${payload.messageCount} messages · ` +
		`system prompt ${payload.systemPromptChars.toLocaleString()} chars · ` +
		`total ${payload.totalChars.toLocaleString()} chars`;
	headerRow.appendChild(headerText);
	var copyBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm");
	copyBtn.textContent = "Copy";
	var downloadBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm");
	downloadBtn.textContent = "Download";
	var llmOutputBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm");
	llmOutputBtn.textContent = "LLM output";
	headerRow.appendChild(copyBtn);
	headerRow.appendChild(downloadBtn);
	headerRow.appendChild(llmOutputBtn);
	var promptMemory = payload.promptMemory || null;
	if (promptMemory?.mode === "frozen-at-session-start") {
		var refreshBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm");
		refreshBtn.textContent = "Refresh memory";
		refreshBtn.addEventListener("click", () => onRefresh(refreshBtn));
		headerRow.appendChild(refreshBtn);
	}
	return { headerRow: headerRow, copyBtn: copyBtn, downloadBtn: downloadBtn, llmOutputBtn: llmOutputBtn };
}

function wireFullContextCopyButton(copyBtn, messages, llmOutputs, llmOutputPanel) {
	copyBtn.addEventListener("click", () => {
		var lines = messages.map((m) => {
			var content = typeof m.content === "string" ? m.content : JSON.stringify(m.content);
			var parts = [content];
			for (var tc of m.tool_calls || []) {
				parts.push(`[tool_call: ${tc.function?.name || "?"} ${tc.function?.arguments || ""}]`);
			}
			return `[${m.role}] ${parts.join("\n")}`;
		});
		var contextText = lines.join("\n");
		var copyText = contextText;
		var llmOutputVisible = llmOutputPanel && !llmOutputPanel.classList.contains("hidden");
		if (llmOutputVisible) {
			copyText = `LLM output:
${JSON.stringify(llmOutputs, null, 2)}

Context:
${contextText}`;
		}
		navigator.clipboard.writeText(copyText).then(() => {
			copyBtn.textContent = "Copied!";
			setTimeout(() => {
				copyBtn.textContent = "Copy";
			}, 1500);
		});
	});
}

function wireFullContextDownloadButton(downloadBtn, messages) {
	downloadBtn.addEventListener("click", () => {
		var lines = [];
		for (var m of messages) {
			lines.push(JSON.stringify(m));
		}
		var blob = new Blob([`${lines.join("\n")}\n`], { type: "application/x-jsonlines" });
		var url = URL.createObjectURL(blob);
		var a = document.createElement("a");
		a.href = url;
		a.download = `context-${new Date().toISOString().slice(0, 19).replace(/[T:]/g, "-")}.jsonl`;
		a.click();
		URL.revokeObjectURL(url);
	});
}

function buildFullContextLlmOutputPanel(llmOutputs) {
	var panel = ctxEl("div", "hidden mb-3");
	var meta = ctxEl(
		"div",
		"text-xs text-[var(--muted)] mb-1",
		`${llmOutputs.length} assistant output${llmOutputs.length === 1 ? "" : "s"}`,
	);
	panel.appendChild(meta);
	var pre = ctxEl(
		"pre",
		"text-xs font-mono whitespace-pre-wrap break-words bg-[var(--surface)] border border-[var(--border)] rounded-md p-2 text-[var(--text)]",
	);
	pre.id = "fullContextLlmOutput";
	pre.textContent = JSON.stringify(llmOutputs, null, 2);
	panel.appendChild(pre);
	return panel;
}

function wireFullContextLlmOutputToggle(button, panel) {
	button.addEventListener("click", () => {
		var hidden = panel.classList.contains("hidden");
		panel.classList.toggle("hidden", !hidden);
		button.textContent = hidden ? "Hide LLM output" : "LLM output";
	});
}

function refreshFullContextMemory(refreshBtn) {
	refreshBtn.disabled = true;
	refreshBtn.textContent = "Refreshing…";
	refreshPromptMemoryToolbarSnapshot().then((promptMemory) => {
		refreshBtn.disabled = false;
		refreshBtn.textContent = "Refresh memory";
		if (promptMemory) return;
	});
}

function refreshFullContextPanel() {
	var panel = S.$("fullContextPanel");
	if (!panel) return;
	panel.textContent = "";
	panel.appendChild(ctxEl("div", "text-xs text-[var(--muted)]", "Building full context\u2026"));

	sendRpc("chat.full_context", {}).then((res) => {
		panel.textContent = "";
		if (!(res?.ok && res.payload)) {
			panel.appendChild(ctxEl("div", "text-xs text-[var(--error)]", "Failed to build context"));
			return;
		}
		var promptMemory = res.payload.promptMemory || null;
		refreshPromptMemoryToolbarFromPayload(promptMemory);
		var promptMemoryBox = buildFullContextPromptMemoryBox(promptMemory);
		if (promptMemoryBox) panel.appendChild(promptMemoryBox);
		appendFullContextWorkspaceWarnings(panel, res.payload);
		var messages = res.payload.messages || [];
		var llmOutputs = res.payload.llmOutputs || [];
		var llmOutputPanel = buildFullContextLlmOutputPanel(llmOutputs);
		var header = buildFullContextHeaderRow(res.payload, refreshFullContextMemory);
		wireFullContextCopyButton(header.copyBtn, messages, llmOutputs, llmOutputPanel);
		wireFullContextDownloadButton(header.downloadBtn, messages);
		wireFullContextLlmOutputToggle(header.llmOutputBtn, llmOutputPanel);
		var headerRow = header.headerRow;
		panel.appendChild(headerRow);
		panel.appendChild(llmOutputPanel);

		for (var i = 0; i < messages.length; i++) {
			panel.appendChild(renderContextMessage(messages[i], i));
		}
	});
}

function toggleFullContextPanel() {
	var modal = S.$("fullContextModal");
	if (!modal) return;
	var opening = modal.classList.contains("hidden");
	if (!opening) {
		setFullContextModalOpen(false);
		return;
	}
	setDebugModalOpen(false);
	setFullContextModalOpen(true);
	refreshFullContextPanel();
}

/** Refresh the full-context panel if it is currently visible. */
export function maybeRefreshFullContext() {
	var modal = S.$("fullContextModal");
	if (modal && !modal.classList.contains("hidden")) refreshFullContextPanel();
}

// ── MCP toggle ───────────────────────────────────────────
export function updateMcpToggleUI(enabled) {
	var btn = S.$("mcpToggleBtn");
	var label = S.$("mcpToggleLabel");
	if (!btn) return;
	if (enabled) {
		btn.style.color = "var(--ok)";
		btn.style.borderColor = "var(--ok)";
		if (label) label.textContent = "MCP";
		btn.title = "MCP tools enabled — click to disable for this session";
	} else {
		btn.style.color = "var(--muted)";
		btn.style.borderColor = "var(--border)";
		if (label) label.textContent = "MCP off";
		btn.title = "MCP tools disabled — click to enable for this session";
	}
}

function toggleMcp() {
	var label = S.$("mcpToggleLabel");
	var isEnabled = label && label.textContent === "MCP";
	var newDisabled = isEnabled;
	sendRpc("sessions.patch", { key: S.activeSessionKey, mcpDisabled: newDisabled }).then((res) => {
		if (res?.ok) {
			updateMcpToggleUI(!newDisabled);
		}
	});
}

// ── Model change notice ──────────────────────────────────
export function showModelNotice(model) {
	if (!S.chatMsgBox) return;
	if (model.supportsTools !== false) return; // Only show for models without tool support

	slashInjectStyles();

	var tpl = document.getElementById("tpl-model-notice");
	if (!tpl) return;

	var card = tpl.content.cloneNode(true).firstElementChild;
	card.querySelector("[data-model-name]").textContent = model.displayName || model.id;
	card.querySelector("[data-provider]").textContent = model.provider || "local";

	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

// ── Slash command handlers ───────────────────────────────
function handleSlashCommand(cmdName, cmdArgs) {
	if (cmdName === "clear") {
		clearActiveSession();
		return;
	}
	if (cmdName === "compact") {
		chatAddMsg("system", "Compacting conversation\u2026");
		sendRpc("chat.compact", {}).then((res) => {
			if (res?.ok) {
				switchSession(S.activeSessionKey);
			} else {
				chatAddMsg("error", res?.error?.message || "Compact failed");
			}
		});
		return;
	}
	if (cmdName === "context") {
		chatAddMsg("system", "Loading context\u2026");
		sendRpc("chat.context", {}).then((res) => {
			if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
			if (res?.ok && res.payload) {
				try {
					renderContextCard(res.payload);
				} catch (err) {
					chatAddMsg("error", `Render error: ${err.message}`);
				}
			} else {
				chatAddMsg("error", res?.error?.message || "Context failed");
			}
		});
		return;
	}
	if (cmdName === "sh") {
		var normalized = (cmdArgs || "").toLowerCase();
		if (normalized === "off" || normalized === "exit") {
			setCommandMode(false);
			chatAddMsg("system", renderMarkdown("**Command:** mode disabled"), true);
			return;
		}
		setCommandMode(true);
		chatAddMsg(
			"system",
			renderMarkdown(`**Command:** mode enabled (${commandModeSummary()}) · exit with /sh off or Esc`),
			true,
		);
	}
}

function tryHandleLocalSlashCommand(text, hasImages) {
	if (text.charAt(0) !== "/" || hasImages) return false;
	var slash = parseSlashCommand(text);
	if (!(slash && shouldHandleSlashLocally(slash.name, slash.args))) return false;
	S.chatInput.value = "";
	chatAutoResize();
	slashHideMenu();
	handleSlashCommand(slash.name, slash.args);
	return true;
}

function rememberChatHistory(text) {
	if (!text) return;
	S.chatHistory.push(text);
	if (S.chatHistory.length > 200) S.setChatHistory(S.chatHistory.slice(-200));
	localStorage.setItem("moltis-chat-history", JSON.stringify(S.chatHistory));
}

function resetComposerAfterSend() {
	S.setChatHistoryIdx(-1);
	S.setChatHistoryDraft("");
	S.chatInput.value = "";
	chatAutoResize();
	if (window.innerWidth < 768) S.chatInput.blur();
}

function normalizeOutgoingText(text, hasImages) {
	if (!(S.commandModeEnabled && text && !hasImages)) return text;
	var parsed = parseSlashCommand(text);
	if (parsed && parsed.name === "sh") return text;
	return `/sh ${text}`;
}

function applySelectedModelToChatParams(chatParams) {
	var effectiveId = modelStore.effectiveModelId.value;
	if (!effectiveId) return;
	chatParams.model = effectiveId;
	setSessionModel(S.activeSessionKey, effectiveId);
}

function handleChatSendRpcResponse(res, userEl) {
	if (res?.ok && res.payload?.runId) {
		setSessionActiveRunId(S.activeSessionKey, res.payload.runId);
	}
	if (res?.payload?.queued) {
		markMessageQueued(userEl, S.activeSessionKey);
		return;
	}
	if (res && !res.ok && res.error) {
		chatAddMsg("error", res.error.message || "Request failed");
	}
}

// ── Build chat params (text-only or multimodal) ─────────
function buildChatMessage(text, seq, displayText) {
	var userText = displayText !== undefined ? displayText : text;
	var images = hasPendingImages() ? getPendingImages() : [];
	if (images.length > 0) {
		var content = [];
		if (text) content.push({ type: "text", text: text });
		for (var img of images) {
			content.push({ type: "image_url", image_url: { url: img.dataUrl } });
		}
		var params = { content: content, _seq: seq };
		var el = chatAddMsgWithImages("user", userText ? renderMarkdown(userText) : "", images);
		clearPendingImages();
		return { params: params, el: el };
	}
	return {
		params: { text: text, _seq: seq },
		el: chatAddMsg("user", renderMarkdown(userText), true),
	};
}

// ── Send chat message ────────────────────────────────────
function sendChat() {
	var text = S.chatInput.value.trim();
	var hasImages = hasPendingImages();
	if (!((text || hasImages) && S.connected)) return;

	// Unlock audio playback while we still have user-gesture context.
	warmAudioPlayback();

	if (tryHandleLocalSlashCommand(text, hasImages)) return;
	rememberChatHistory(text);
	resetComposerAfterSend();
	var outgoingText = normalizeOutgoingText(text, hasImages);

	S.setChatSeq(S.chatSeq + 1);
	var msg = buildChatMessage(outgoingText, S.chatSeq, text);
	var chatParams = msg.params;
	var userEl = msg.el;
	// Highlight code blocks in the user message (if any).
	if (userEl) highlightCodeBlocks(userEl);

	applySelectedModelToChatParams(chatParams);
	bumpSessionCount(S.activeSessionKey, 1);
	cacheOutgoingUserMessage(S.activeSessionKey, chatParams);
	seedSessionPreviewFromUserText(S.activeSessionKey, text || outgoingText);
	setSessionReplying(S.activeSessionKey, true);
	sendRpc("chat.send", chatParams).then((res) => handleChatSendRpcResponse(res, userEl));
	maybeRefreshFullContext();
}

function markMessageQueued(el, sessionKey) {
	if (!el) return;
	var tray = document.getElementById("queuedMessages");
	if (!tray) return;
	console.debug("[queued] marking user message as queued, moving to tray", { sessionKey });
	// Move the user message from the main chat into the queued tray.
	el.classList.add("queued");
	var badge = document.createElement("div");
	badge.className = "queued-badge";
	var label = document.createElement("span");
	label.className = "queued-label";
	label.textContent = "Queued";
	var btn = document.createElement("button");
	btn.className = "queued-cancel";
	btn.title = "Cancel all queued";
	btn.textContent = "\u2715";
	btn.addEventListener("click", (e) => {
		e.stopPropagation();
		sendRpc("chat.cancel_queued", { sessionKey });
	});
	badge.appendChild(label);
	badge.appendChild(btn);
	el.appendChild(badge);
	tray.appendChild(el);
	tray.classList.remove("hidden");
}

function chatAutoResize() {
	if (!S.chatInput) return;
	S.chatInput.style.height = "auto";
	S.chatInput.style.height = `${Math.min(S.chatInput.scrollHeight, 120)}px`;
}

// ── History navigation helpers ───────────────────────────
function handleHistoryUp() {
	if (S.chatHistory.length === 0) return;
	if (S.chatHistoryIdx === -1) {
		S.setChatHistoryDraft(S.chatInput.value);
		S.setChatHistoryIdx(S.chatHistory.length - 1);
	} else if (S.chatHistoryIdx > 0) {
		S.setChatHistoryIdx(S.chatHistoryIdx - 1);
	}
	S.chatInput.value = S.chatHistory[S.chatHistoryIdx];
	chatAutoResize();
}

function handleHistoryDown() {
	if (S.chatHistoryIdx === -1) return;
	if (S.chatHistoryIdx < S.chatHistory.length - 1) {
		S.setChatHistoryIdx(S.chatHistoryIdx + 1);
		S.chatInput.value = S.chatHistory[S.chatHistoryIdx];
	} else {
		S.setChatHistoryIdx(-1);
		S.chatInput.value = S.chatHistoryDraft;
	}
	chatAutoResize();
}

// Safe: static hardcoded HTML template string — no user input is interpolated.
var chatPageHTML =
	'<div style="position:absolute;inset:0;display:grid;grid-template-rows:auto auto 1fr auto auto auto;overflow:hidden">' +
	'<div class="chat-toolbar h-12 px-4 border-b border-[var(--border)] bg-[var(--surface)] flex items-center gap-2" style="grid-row:1;">' +
	'<div id="modelCombo" class="model-combo">' +
	'<button id="modelComboBtn" class="model-combo-btn" type="button">' +
	'<span id="modelComboLabel">loading\u2026</span>' +
	'<span class="icon icon-sm icon-chevron-down model-combo-chevron"></span>' +
	"</button>" +
	'<div id="modelDropdown" class="model-dropdown hidden">' +
	'<input id="modelSearchInput" type="text" placeholder="Search models\u2026" class="model-search-input" autocomplete="off" />' +
	'<div id="modelDropdownList" class="model-dropdown-list"></div>' +
	"</div>" +
	"</div>" +
	'<div id="reasoningCombo" class="model-combo hidden">' +
	'<button id="reasoningComboBtn" class="model-combo-btn" type="button" title="Reasoning effort">' +
	'<span class="icon icon-sm icon-brain" style="flex-shrink:0;"></span>' +
	'<span id="reasoningComboLabel">Off</span>' +
	'<span class="icon icon-sm icon-chevron-down model-combo-chevron"></span>' +
	"</button>" +
	'<div id="reasoningDropdown" class="model-dropdown hidden">' +
	'<div id="reasoningDropdownList" class="model-dropdown-list"></div>' +
	"</div>" +
	"</div>" +
	'<div id="nodeCombo" class="model-combo hidden">' +
	'<button id="nodeComboBtn" class="model-combo-btn" type="button">' +
	'<span class="icon icon-sm icon-server" style="flex-shrink:0;"></span>' +
	'<span id="nodeComboLabel">Local</span>' +
	'<span class="icon icon-sm icon-chevron-down model-combo-chevron"></span>' +
	"</button>" +
	'<div id="nodeDropdown" class="model-dropdown hidden" tabindex="-1">' +
	'<div id="nodeDropdownList" class="model-dropdown-list"></div>' +
	"</div>" +
	"</div>" +
	'<div id="sessionHeaderToolbarMount" class="ml-auto flex items-center gap-1.5"></div>' +
	'<button id="chatMoreBtn" type="button" class="model-combo-btn" title="More controls" aria-label="More controls">' +
	'<span class="icon icon-lg icon-menu-dots-horizontal"></span>' +
	"</button>" +
	"</div>" +
	'<div id="chatMoreModal" class="provider-modal-backdrop hidden">' +
	'<div class="provider-modal" style="width:560px;max-width:92vw;">' +
	'<div class="provider-modal-header">' +
	'<div class="flex items-center gap-2">' +
	'<button id="chatMoreDeleteAllBtn" type="button" class="provider-btn provider-btn-sm chat-session-btn-danger inline-flex items-center gap-1.5" style="background:var(--error);border-color:var(--error);color:#fff;">' +
	'<span class="icon icon-sm icon-x-circle shrink-0"></span>' +
	'<span id="chatMoreDeleteAllLabel">Delete all sessions</span>' +
	"</button>" +
	"</div>" +
	'<div id="sessionHeaderModalTopMount" class="flex items-center gap-2"></div>' +
	"</div>" +
	'<div class="provider-modal-body flex flex-col gap-3">' +
	'<div class="flex flex-wrap items-center gap-2">' +
	'<button id="sandboxToggle" class="sandbox-toggle text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1" title="Toggle sandbox mode">' +
	'<span class="icon icon-md icon-lock shrink-0"></span>' +
	'<span id="sandboxLabel">sandboxed</span>' +
	"</button>" +
	'<div style="position:relative;display:inline-block">' +
	'<button id="sandboxImageBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Sandbox image">' +
	'<span class="icon icon-md icon-cube shrink-0"></span>' +
	'<span id="sandboxImageLabel" class="max-w-[120px] truncate">ubuntu:25.10</span>' +
	"</button>" +
	'<div id="sandboxImageDropdown" class="hidden" style="position:absolute;top:100%;left:0;z-index:50;margin-top:4px;min-width:200px;max-height:300px;overflow-y:auto;background:var(--surface);border:1px solid var(--border);border-radius:8px;box-shadow:0 4px 12px rgba(0,0,0,.15);"></div>' +
	"</div>" +
	'<button id="mcpToggleBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1" title="Toggle MCP tools for this session">' +
	'<span class="icon icon-md icon-link shrink-0"></span>' +
	'<span id="mcpToggleLabel">MCP</span>' +
	"</button>" +
	'<button id="debugPanelBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Show context debug info">' +
	'<span class="icon icon-md icon-wrench shrink-0"></span>' +
	'<span id="debugPanelLabel">Debug</span>' +
	"</button>" +
	'<button id="fullContextBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Show full LLM context (system prompt + history)">' +
	'<span class="icon icon-md icon-document shrink-0"></span>' +
	'<span id="fullContextLabel">Context</span>' +
	"</button>" +
	"</div>" +
	'<div id="sessionControlsSection" class="border-t border-[var(--border)] pt-3">' +
	'<div id="sessionHeaderModalMount" class="w-full"></div>' +
	"</div>" +
	"</div>" +
	"</div>" +
	"</div>" +
	'<div id="debugModal" class="provider-modal-backdrop hidden">' +
	'<div class="provider-modal" style="width:min(980px,96vw);max-width:96vw;max-height:88vh;">' +
	'<div class="provider-modal-header">' +
	'<div class="provider-item-name">Debug context</div>' +
	'<button id="debugModalCloseBtn" type="button" class="provider-btn provider-btn-secondary provider-btn-sm">Close</button>' +
	"</div>" +
	'<div class="provider-modal-body" style="padding:0;overflow:hidden;">' +
	'<div id="debugPanel" class="px-4 py-3 overflow-y-auto" style="max-height:72vh;"></div>' +
	"</div>" +
	"</div>" +
	"</div>" +
	'<div id="fullContextModal" class="provider-modal-backdrop hidden">' +
	'<div class="provider-modal" style="width:min(1080px,96vw);max-width:96vw;max-height:88vh;">' +
	'<div class="provider-modal-header">' +
	'<div class="provider-item-name">Full context</div>' +
	'<button id="fullContextModalCloseBtn" type="button" class="provider-btn provider-btn-secondary provider-btn-sm">Close</button>' +
	"</div>" +
	'<div class="provider-modal-body" style="padding:0;overflow:hidden;">' +
	'<div id="fullContextPanel" class="px-4 py-3 overflow-y-auto" style="max-height:72vh;"></div>' +
	"</div>" +
	"</div>" +
	"</div>" +
	'<div class="p-4 flex flex-col gap-2" id="messages" style="grid-row:3;overflow-y:auto;min-height:0"></div>' +
	'<div id="queuedMessages" class="queued-tray hidden" style="grid-row:4;"></div>' +
	'<div id="tokenBar" class="token-bar" style="grid-row:5;"></div>' +
	'<div class="chat-input-row px-4 py-3 border-t border-[var(--border)] bg-[var(--surface)] flex gap-2 items-end" style="grid-row:6;">' +
	'<span id="chatCommandPrompt" class="chat-command-prompt chat-command-prompt-hidden" title="Command prompt symbol" aria-hidden="true">$</span>' +
	'<textarea id="chatInput" placeholder="Type a message..." rows="1" enterkeyhint="send" ' +
	'class="flex-1 bg-[var(--surface2)] border border-[var(--border)] text-[var(--text)] px-3 py-2 rounded-lg text-sm resize-none min-h-[40px] max-h-[120px] leading-relaxed focus:outline-none focus:border-[var(--border-strong)] focus:ring-1 focus:ring-[var(--accent-subtle)] transition-colors font-[var(--font-body)]"></textarea>' +
	'<button id="micBtn" disabled title="Click to start recording" ' +
	'class="mic-btn min-h-[40px] px-3 bg-[var(--surface2)] border border-[var(--border)] rounded-lg text-[var(--muted)] cursor-pointer disabled:opacity-40 disabled:cursor-default transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text)]">' +
	'<span class="icon icon-lg icon-microphone"></span>' +
	"</button>" +
	'<button id="sendBtn" disabled ' +
	'class="provider-btn min-h-[40px] disabled:opacity-40 disabled:cursor-default">Send</button>' +
	"</div></div>";

function msgRole(el) {
	if (el.classList.contains("user")) return "You";
	if (el.classList.contains("assistant")) return "Assistant";
	return null;
}

/** Intercept copy to prepend role labels when multiple messages are selected. */
function handleChatCopy(e) {
	var sel = window.getSelection();
	if (!sel || sel.isCollapsed || !S.chatMsgBox) return;

	var lines = [];
	for (var msg of S.chatMsgBox.querySelectorAll(".msg")) {
		if (!sel.containsNode(msg, true)) continue;
		var role = msgRole(msg);
		if (!role) continue;
		var text = sel.containsNode(msg, false) ? msg.textContent.trim() : sel.toString().trim();
		if (text) lines.push(`${role}:\n${text}`);
	}
	if (lines.length > 1) {
		e.preventDefault();
		e.clipboardData.setData("text/plain", lines.join("\n\n"));
	}
}

function mountSessionHeaderControls(closeChatMore) {
	var headerToolbarMount = S.$("sessionHeaderToolbarMount");
	if (headerToolbarMount) {
		render(
			html`<${SessionHeader}
					showName=${false}
					showShare=${false}
					showFork=${false}
					showClear=${false}
					showDelete=${false}
					showArchive=${false}
				/>`,
			headerToolbarMount,
		);
	}
	var headerModalMount = S.$("sessionHeaderModalMount");
	if (headerModalMount) {
		render(
			html`<${SessionHeader}
					showSelectors=${false}
					showStop=${false}
					showFork=${false}
					showShare=${false}
					showDelete=${false}
					showArchive=${false}
					nameOwnLine=${true}
					showRenameButton=${true}
				/>`,
			headerModalMount,
		);
	}
	var headerModalTopMount = S.$("sessionHeaderModalTopMount");
	if (headerModalTopMount) {
		render(
			html`<${SessionHeader}
					showSelectors=${false}
					showName=${false}
					showStop=${false}
					actionButtonClass=${"provider-btn provider-btn-secondary provider-btn-sm"}
					onBeforeShare=${() => closeChatMore?.()}
					onBeforeArchive=${() => closeChatMore?.()}
					onBeforeDelete=${() => closeChatMore?.()}
				/>`,
			headerModalTopMount,
		);
	}
}

function bindSessionControlsVisibility() {
	var sessionControlsSection = S.$("sessionControlsSection");
	if (!sessionControlsSection) return;
	disposeSessionControlsVisibility?.();
	disposeSessionControlsVisibility = effect(() => {
		var isMainSession = (sessionStore.activeSessionKey.value || "main") === "main";
		sessionControlsSection.classList.toggle("hidden", isMainSession);
	});
}

function bindChatMoreModal(debugModal, fullContextModal, closeDebugModal, closeFullContextModal) {
	var chatMoreModal = S.$("chatMoreModal");
	var chatMoreBtn = S.$("chatMoreBtn");
	if (!(chatMoreModal && chatMoreBtn)) return null;
	var closeChatMore = () => {
		chatMoreModal.classList.add("hidden");
		chatMoreBtn.classList.remove("active");
		if (S.sandboxImageDropdown) {
			S.sandboxImageDropdown.classList.add("hidden");
		}
	};
	var openChatMore = () => {
		setDebugModalOpen(false);
		setFullContextModalOpen(false);
		chatMoreModal.classList.remove("hidden");
		chatMoreBtn.classList.add("active");
	};
	chatMoreBtn.addEventListener("click", openChatMore);
	chatMoreModal.addEventListener("click", (e) => {
		if (e.target === chatMoreModal) closeChatMore();
	});
	for (var closeAfterToggleId of ["debugPanelBtn", "fullContextBtn"]) {
		var closeAfterToggleBtn = S.$(closeAfterToggleId);
		if (closeAfterToggleBtn) closeAfterToggleBtn.addEventListener("click", closeChatMore);
	}
	chatMoreModalKeydownHandler = (e) => {
		if (e.key !== "Escape") return;
		if (fullContextModal && !fullContextModal.classList.contains("hidden")) {
			closeFullContextModal?.();
			return;
		}
		if (debugModal && !debugModal.classList.contains("hidden")) {
			closeDebugModal?.();
			return;
		}
		closeChatMore();
	};
	document.addEventListener("keydown", chatMoreModalKeydownHandler);
	return closeChatMore;
}

function bindDeleteAllSessions(closeChatMore) {
	var chatMoreDeleteAllBtn = S.$("chatMoreDeleteAllBtn");
	if (!chatMoreDeleteAllBtn) return;
	var chatMoreDeleteAllLabel = S.$("chatMoreDeleteAllLabel");
	var deleteAllInFlight = false;
	chatMoreDeleteAllBtn.addEventListener("click", () => {
		if (deleteAllInFlight) return;
		deleteAllInFlight = true;
		chatMoreDeleteAllBtn.disabled = true;
		if (chatMoreDeleteAllLabel) {
			chatMoreDeleteAllLabel.textContent = "Deleting…";
		}
		closeChatMore?.();
		clearAllSessions()
			.then((res) => {
				if (res?.ok && !res?.skipped) return;
				if (res?.cancelled || res?.skipped) return;
				chatAddMsg("error", res?.error?.message || "Failed to clear sessions");
			})
			.finally(() => {
				deleteAllInFlight = false;
				chatMoreDeleteAllBtn.disabled = false;
				if (chatMoreDeleteAllLabel) {
					chatMoreDeleteAllLabel.textContent = "Delete all sessions";
				}
			});
	});
}

function bindChatComposer() {
	S.chatInput.addEventListener("input", () => {
		chatAutoResize();
		slashHandleInput();
	});
	S.chatInput.addEventListener("keydown", (e) => {
		if (slashHandleKeydown(e)) return;
		if (e.key === "Escape" && S.commandModeEnabled && !S.chatInput.value.trim()) {
			e.preventDefault();
			setCommandMode(false);
			return;
		}
		if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
			e.preventDefault();
			sendChat();
			return;
		}
		if (e.key === "ArrowUp" && S.chatInput.selectionStart === 0 && !e.shiftKey) {
			e.preventDefault();
			handleHistoryUp();
			return;
		}
		if (e.key === "ArrowDown" && S.chatInput.selectionStart === S.chatInput.value.length && !e.shiftKey) {
			e.preventDefault();
			handleHistoryDown();
		}
	});
	S.chatSendBtn.addEventListener("click", sendChat);
}

function initializeChatControls() {
	S.setModelCombo(S.$("modelCombo"));
	S.setModelComboBtn(S.$("modelComboBtn"));
	S.setModelComboLabel(S.$("modelComboLabel"));
	S.setModelDropdown(S.$("modelDropdown"));
	S.setModelSearchInput(S.$("modelSearchInput"));
	S.setModelDropdownList(S.$("modelDropdownList"));
	bindModelComboEvents();

	bindReasoningToggle();

	S.setNodeCombo(S.$("nodeCombo"));
	S.setNodeComboBtn(S.$("nodeComboBtn"));
	S.setNodeComboLabel(S.$("nodeComboLabel"));
	S.setNodeDropdown(S.$("nodeDropdown"));
	S.setNodeDropdownList(S.$("nodeDropdownList"));
	bindNodeComboEvents();
	fetchNodes();

	S.setSandboxToggleBtn(S.$("sandboxToggle"));
	S.setSandboxLabel(S.$("sandboxLabel"));
	bindSandboxToggleEvents();
	updateSandboxUI(true);

	S.setSandboxImageBtn(S.$("sandboxImageBtn"));
	S.setSandboxImageLabel(S.$("sandboxImageLabel"));
	S.setSandboxImageDropdown(S.$("sandboxImageDropdown"));
	bindSandboxImageEvents();
	updateSandboxImageUI(null);
}

function bindContextModals() {
	var debugModal = S.$("debugModal");
	var debugModalCloseBtn = S.$("debugModalCloseBtn");
	var closeDebugModal = null;
	if (debugModal) {
		closeDebugModal = () => setDebugModalOpen(false);
		if (debugModalCloseBtn) debugModalCloseBtn.addEventListener("click", closeDebugModal);
		debugModal.addEventListener("click", (e) => {
			if (e.target === debugModal) closeDebugModal();
		});
	}

	var fullContextModal = S.$("fullContextModal");
	var fullContextModalCloseBtn = S.$("fullContextModalCloseBtn");
	var closeFullContextModal = null;
	if (fullContextModal) {
		closeFullContextModal = () => setFullContextModalOpen(false);
		if (fullContextModalCloseBtn) fullContextModalCloseBtn.addEventListener("click", closeFullContextModal);
		fullContextModal.addEventListener("click", (e) => {
			if (e.target === fullContextModal) closeFullContextModal();
		});
	}

	return {
		debugModal: debugModal,
		fullContextModal: fullContextModal,
		closeDebugModal: closeDebugModal,
		closeFullContextModal: closeFullContextModal,
	};
}

function syncModelComboLabel() {
	if (!(S.models.length > 0 && S.modelComboLabel)) return;
	var found = S.models.find((m) => m.id === S.selectedModelId);
	if (found) {
		S.modelComboLabel.textContent = found.displayName || found.id;
		return;
	}
	if (S.models[0]) {
		S.modelComboLabel.textContent = S.models[0].displayName || S.models[0].id;
	}
}

function resolveInitialSessionKey(sessionKeyFromUrl) {
	if (sessionKeyFromUrl) return sessionKeyFromUrl;
	var sessionKey = localStorage.getItem("moltis-session") || "main";
	history.replaceState(null, "", sessionPath(sessionKey));
	return sessionKey;
}

function startInitialChatSession(sessionKey) {
	if (!S.connected) return;
	S.chatSendBtn.disabled = false;
	switchSession(sessionKey);
}

function initializeChatMediaDrop() {
	if (window.innerWidth < 768) return;
	var inputArea = S.chatInput?.closest(".px-4.py-3");
	initMediaDrop(S.chatMsgBox, inputArea);
}

registerPrefix(
	routes.chats,
	function initChat(container, sessionKeyFromUrl) {
		container.style.cssText = "position:relative";
		// Safe: chatPageHTML is a static hardcoded template with no user input.
		// This is a compile-time constant defined above — no dynamic or user data.
		container.innerHTML = chatPageHTML; // eslint-disable-line no-unsanitized/property

		S.setChatMsgBox(S.$("messages"));
		S.setChatInput(S.$("chatInput"));
		S.setChatSendBtn(S.$("sendBtn"));
		updateCommandInputUI();
		initializeChatControls();

		var closeChatMore = null;
		mountSessionHeaderControls(() => closeChatMore?.());
		bindSessionControlsVisibility();

		var mcpToggle = S.$("mcpToggleBtn");
		if (mcpToggle) mcpToggle.addEventListener("click", toggleMcp);
		updateMcpToggleUI(true); // default: MCP enabled

		var modalBindings = bindContextModals();
		closeChatMore = bindChatMoreModal(
			modalBindings.debugModal,
			modalBindings.fullContextModal,
			modalBindings.closeDebugModal,
			modalBindings.closeFullContextModal,
		);
		bindDeleteAllSessions(closeChatMore);

		var debugBtn = S.$("debugPanelBtn");
		if (debugBtn) debugBtn.addEventListener("click", toggleDebugPanel);

		S.$("fullContextBtn")?.addEventListener("click", toggleFullContextPanel);

		syncModelComboLabel();
		var sessionKey = resolveInitialSessionKey(sessionKeyFromUrl);
		startInitialChatSession(sessionKey);

		bindChatComposer();

		S.chatMsgBox.addEventListener("copy", handleChatCopy);

		// Initialize voice input
		initVoiceInput(S.$("micBtn"));

		initializeChatMediaDrop();

		S.chatInput.focus();
	},
	function teardownChat() {
		teardownVoiceInput();
		teardownMediaDrop();
		unbindReasoningToggle();
		unbindNodeEvents();
		slashHideMenu();
		if (chatMoreModalKeydownHandler) {
			document.removeEventListener("keydown", chatMoreModalKeydownHandler);
			chatMoreModalKeydownHandler = null;
		}
		disposeSessionControlsVisibility?.();
		disposeSessionControlsVisibility = null;
		var headerToolbarMount = S.$("sessionHeaderToolbarMount");
		if (headerToolbarMount) render(null, headerToolbarMount);
		var headerModalMount = S.$("sessionHeaderModalMount");
		if (headerModalMount) render(null, headerModalMount);
		var headerModalTopMount = S.$("sessionHeaderModalTopMount");
		if (headerModalTopMount) render(null, headerModalTopMount);
		S.setChatMsgBox(null);
		S.setChatInput(null);
		S.setChatSendBtn(null);
		S.setStreamEl(null);
		S.setStreamText("");
		S.setModelCombo(null);
		S.setModelComboBtn(null);
		S.setModelComboLabel(null);
		S.setModelDropdown(null);
		S.setModelSearchInput(null);
		S.setModelDropdownList(null);
		S.setNodeCombo(null);
		S.setNodeComboBtn(null);
		S.setNodeComboLabel(null);
		S.setNodeDropdown(null);
		S.setNodeDropdownList(null);
		S.setSandboxToggleBtn(null);
		S.setSandboxLabel(null);
	},
);
