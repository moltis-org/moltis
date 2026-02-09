// ── Chat page ────────────────────────────────────────────

import { chatAddMsg, chatAddMsgWithImages, updateTokenBar } from "./chat-ui.js";
import { formatBytes, formatTokens, renderMarkdown, sendRpc } from "./helpers.js";
import {
	clearPendingImages,
	getPendingImages,
	hasPendingImages,
	initMediaDrop,
	teardownMediaDrop,
} from "./media-drop.js";
import { bindModelComboEvents, setSessionModel } from "./models.js";
import { registerPrefix, sessionPath } from "./router.js";
import { bindSandboxImageEvents, bindSandboxToggleEvents, updateSandboxImageUI, updateSandboxUI } from "./sandbox.js";
import {
	bumpSessionCount,
	fetchSessions,
	setSessionReplying,
	switchSession,
	updateChatSessionHeader,
} from "./sessions.js";
import * as S from "./state.js";
import { initVoiceInput, teardownVoiceInput } from "./voice-input.js";

// ── Slash commands ───────────────────────────────────────
var slashCommands = [
	{ name: "clear", description: "Clear conversation history" },
	{ name: "compact", description: "Summarize conversation to save tokens" },
	{ name: "context", description: "Show session context and project info" },
];
var slashMenuEl = null;
var slashMenuIdx = 0;
var slashMenuItems = [];

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
		".ctx-header svg{flex-shrink:0;opacity:.7}" +
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
		".ctx-warning svg{flex-shrink:0;color:var(--warning,#eab308)}" +
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
	var sandboxSection = ctxSection("Sandbox");
	sandboxSection.appendChild(ctxRow("Enabled", sb.enabled ? "yes" : "no", true));
	if (sb.backend) {
		sandboxSection.appendChild(ctxRow("Backend", sb.backend));
		if (sb.mode) sandboxSection.appendChild(ctxRow("Mode", sb.mode));
		if (sb.scope) sandboxSection.appendChild(ctxRow("Scope", sb.scope));
		if (sb.workspaceMount) sandboxSection.appendChild(ctxRow("Workspace Mount", sb.workspaceMount));
		if (sb.image) sandboxSection.appendChild(ctxRow("Image", sb.image, true));
		if (sb.containerName) sandboxSection.appendChild(ctxRow("Container", sb.containerName));
	}
	card.appendChild(sandboxSection);
}

function renderContextTokensSection(card, data) {
	var tu = data.tokenUsage || {};
	var tokenSection = ctxSection("Token Usage");
	tokenSection.appendChild(ctxRow("Input", formatTokens(tu.inputTokens || 0), true));
	tokenSection.appendChild(ctxRow("Output", formatTokens(tu.outputTokens || 0), true));
	tokenSection.appendChild(ctxRow("Total", formatTokens(tu.total || 0), true));
	if (tu.contextWindow > 0) {
		var pct = Math.max(0, 100 - Math.round(((tu.total || 0) / tu.contextWindow) * 100));
		tokenSection.appendChild(ctxRow("Context left", `${pct}% of ${formatTokens(tu.contextWindow)}`, true));
	}
	card.appendChild(tokenSection);
}

function renderContextCard(data) {
	if (!S.chatMsgBox) return;
	slashInjectStyles();

	var card = ctxEl("div", "ctx-card");

	var header = ctxEl("div", "ctx-header");
	var svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
	svg.setAttribute("width", "16");
	svg.setAttribute("height", "16");
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("fill", "none");
	svg.setAttribute("stroke", "currentColor");
	svg.setAttribute("stroke-width", "2");
	svg.setAttribute("stroke-linecap", "round");
	svg.setAttribute("stroke-linejoin", "round");
	var path1 = document.createElementNS("http://www.w3.org/2000/svg", "circle");
	path1.setAttribute("cx", "12");
	path1.setAttribute("cy", "12");
	path1.setAttribute("r", "3");
	var path2 = document.createElementNS("http://www.w3.org/2000/svg", "path");
	path2.setAttribute(
		"d",
		"M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
	);
	svg.appendChild(path1);
	svg.appendChild(path2);
	header.appendChild(svg);
	header.appendChild(ctxEl("span", "ctx-header-title", "Context"));
	card.appendChild(header);

	// Show warning if tools are disabled
	if (data.supportsTools === false) {
		var warning = ctxEl("div", "ctx-warning");
		var warnSvg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
		warnSvg.setAttribute("width", "16");
		warnSvg.setAttribute("height", "16");
		warnSvg.setAttribute("viewBox", "0 0 24 24");
		warnSvg.setAttribute("fill", "none");
		warnSvg.setAttribute("stroke", "currentColor");
		warnSvg.setAttribute("stroke-width", "2");
		warnSvg.setAttribute("stroke-linecap", "round");
		warnSvg.setAttribute("stroke-linejoin", "round");
		var tri = document.createElementNS("http://www.w3.org/2000/svg", "path");
		tri.setAttribute("d", "M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z");
		var line1 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		line1.setAttribute("x1", "12");
		line1.setAttribute("y1", "9");
		line1.setAttribute("x2", "12");
		line1.setAttribute("y2", "13");
		var line2 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		line2.setAttribute("x1", "12");
		line2.setAttribute("y1", "17");
		line2.setAttribute("x2", "12.01");
		line2.setAttribute("y2", "17");
		warnSvg.appendChild(tri);
		warnSvg.appendChild(line1);
		warnSvg.appendChild(line2);
		warning.appendChild(warnSvg);
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
	renderContextTokensSection(card, data);

	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

export function renderCompactCard(data) {
	if (!S.chatMsgBox) return;
	slashInjectStyles();

	var card = ctxEl("div", "ctx-card");

	var header = ctxEl("div", "ctx-header");
	var svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
	svg.setAttribute("width", "16");
	svg.setAttribute("height", "16");
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("fill", "none");
	svg.setAttribute("stroke", "currentColor");
	svg.setAttribute("stroke-width", "2");
	svg.setAttribute("stroke-linecap", "round");
	svg.setAttribute("stroke-linejoin", "round");
	var p1 = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
	p1.setAttribute("points", "4 14 10 14 10 20");
	var p2 = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
	p2.setAttribute("points", "20 10 14 10 14 4");
	var l1 = document.createElementNS("http://www.w3.org/2000/svg", "line");
	l1.setAttribute("x1", "14");
	l1.setAttribute("y1", "10");
	l1.setAttribute("x2", "21");
	l1.setAttribute("y2", "3");
	var l2 = document.createElementNS("http://www.w3.org/2000/svg", "line");
	l2.setAttribute("x1", "3");
	l2.setAttribute("y1", "21");
	l2.setAttribute("x2", "10");
	l2.setAttribute("y2", "14");
	svg.appendChild(p1);
	svg.appendChild(p2);
	svg.appendChild(l1);
	svg.appendChild(l2);
	header.appendChild(svg);
	header.appendChild(ctxEl("span", "ctx-header-title", "Conversation compacted"));
	card.appendChild(header);

	var statsSection = ctxSection("Before compact");
	statsSection.appendChild(ctxRow("Messages", String(data.messageCount || 0)));
	statsSection.appendChild(ctxRow("Total tokens", formatTokens(data.totalTokens || 0)));
	if (data.contextWindow) {
		var pctUsed = Math.round(((data.totalTokens || 0) / data.contextWindow) * 100);
		statsSection.appendChild(ctxRow("Context usage", `${pctUsed}% of ${formatTokens(data.contextWindow)}`));
	}
	card.appendChild(statsSection);

	var afterSection = ctxSection("After compact");
	afterSection.appendChild(ctxRow("Messages", "1 (summary)"));
	afterSection.appendChild(ctxRow("Status", "Conversation history replaced with a summary"));
	card.appendChild(afterSection);

	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

// ── Debug panel ──────────────────────────────────────────
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
		renderContextTokensSection(panel, res.payload);
	});
}

function toggleDebugPanel() {
	var panel = S.$("debugPanel");
	var btn = S.$("debugPanelBtn");
	if (!panel) return;
	var hidden = panel.classList.contains("hidden");
	panel.classList.toggle("hidden", !hidden);
	if (btn) btn.style.color = hidden ? "var(--accent)" : "var(--muted)";
	if (hidden) refreshDebugPanel();
}

// ── Raw prompt panel ─────────────────────────────────────

function refreshRawPromptPanel() {
	var panel = S.$("rawPromptPanel");
	if (!panel) return;
	panel.textContent = "";
	panel.appendChild(ctxEl("div", "text-xs text-[var(--muted)]", "Building prompt\u2026"));

	sendRpc("chat.raw_prompt", {}).then((res) => {
		panel.textContent = "";
		if (!(res?.ok && res.payload)) {
			panel.appendChild(ctxEl("div", "text-xs text-[var(--error)]", "Failed to build prompt"));
			return;
		}
		var header = ctxEl("div", "text-xs text-[var(--muted)] mb-2");
		header.textContent = `Full system prompt sent to the model · ${res.payload.charCount} chars · ${res.payload.toolCount} tools · native_tools=${res.payload.native_tools}`;
		panel.appendChild(header);

		var pre = ctxEl(
			"pre",
			"text-xs font-mono whitespace-pre-wrap break-words bg-[var(--surface)] border border-[var(--border)] rounded-md p-3 overflow-y-auto text-[var(--text)]",
		);
		pre.style.maxHeight = "320px";
		pre.textContent = res.payload.prompt;
		panel.appendChild(pre);
	});
}

function toggleRawPromptPanel() {
	var panel = S.$("rawPromptPanel");
	var btn = S.$("rawPromptBtn");
	if (!panel) return;
	var hidden = panel.classList.contains("hidden");
	panel.classList.toggle("hidden", !hidden);
	if (btn) btn.style.color = hidden ? "var(--accent)" : "var(--muted)";
	if (hidden) refreshRawPromptPanel();
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
		var headerRow = ctxEl("div", "flex items-center gap-3 mb-3");
		var headerText = ctxEl("span", "text-xs text-[var(--muted)]");
		headerText.textContent =
			`${res.payload.messageCount} messages \xb7 ` +
			`system prompt ${res.payload.systemPromptChars.toLocaleString()} chars \xb7 ` +
			`total ${res.payload.totalChars.toLocaleString()} chars`;
		headerRow.appendChild(headerText);

		var messages = res.payload.messages || [];

		var copyBtn = ctxEl("button", "provider-btn provider-btn-secondary text-xs");
		copyBtn.textContent = "Copy";
		copyBtn.addEventListener("click", () => {
			var lines = messages.map((m) => {
				var content = typeof m.content === "string" ? m.content : JSON.stringify(m.content);
				var parts = [content];
				for (var tc of m.tool_calls || []) {
					parts.push(`[tool_call: ${tc.function?.name || "?"} ${tc.function?.arguments || ""}]`);
				}
				return `[${m.role}] ${parts.join("\n")}`;
			});
			navigator.clipboard.writeText(lines.join("\n")).then(() => {
				copyBtn.textContent = "Copied!";
				setTimeout(() => {
					copyBtn.textContent = "Copy";
				}, 1500);
			});
		});
		headerRow.appendChild(copyBtn);
		panel.appendChild(headerRow);

		for (var i = 0; i < messages.length; i++) {
			panel.appendChild(renderContextMessage(messages[i], i));
		}
	});
}

function toggleFullContextPanel() {
	var panel = S.$("fullContextPanel");
	var btn = S.$("fullContextBtn");
	if (!panel) return;
	var hidden = panel.classList.contains("hidden");
	panel.classList.toggle("hidden", !hidden);
	if (btn) btn.style.color = hidden ? "var(--accent)" : "var(--muted)";
	if (hidden) refreshFullContextPanel();
}

/** Refresh the full-context panel if it is currently visible. */
export function maybeRefreshFullContext() {
	var panel = S.$("fullContextPanel");
	if (panel && !panel.classList.contains("hidden")) refreshFullContextPanel();
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
	sendRpc("sessions.patch", { key: S.activeSessionKey, mcp_disabled: newDisabled }).then((res) => {
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
function handleSlashCommand(cmdName) {
	if (cmdName === "clear") {
		sendRpc("chat.clear", {}).then((res) => {
			if (res?.ok) {
				if (S.chatMsgBox) S.chatMsgBox.textContent = "";
				S.setSessionTokens({ input: 0, output: 0 });
				updateTokenBar();
				fetchSessions();
			} else {
				chatAddMsg("error", res?.error?.message || "Clear failed");
			}
		});
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
	}
}

// ── Build chat params (text-only or multimodal) ─────────
function buildChatMessage(text, seq) {
	var images = hasPendingImages() ? getPendingImages() : [];
	if (images.length > 0) {
		var content = [];
		if (text) content.push({ type: "text", text: text });
		for (var img of images) {
			content.push({ type: "image_url", image_url: { url: img.dataUrl } });
		}
		var params = { content: content, _seq: seq };
		var el = chatAddMsgWithImages("user", text ? renderMarkdown(text) : "", images);
		clearPendingImages();
		return { params: params, el: el };
	}
	return {
		params: { text: text, _seq: seq },
		el: chatAddMsg("user", renderMarkdown(text), true),
	};
}

// ── Send chat message ────────────────────────────────────
function sendChat() {
	var text = S.chatInput.value.trim();
	var hasImages = hasPendingImages();
	if (!((text || hasImages) && S.connected)) return;

	if (text.charAt(0) === "/" && !hasImages) {
		var cmdName = text.substring(1).toLowerCase();
		var matched = slashCommands.find((c) => c.name === cmdName);
		if (matched) {
			S.chatInput.value = "";
			chatAutoResize();
			slashHideMenu();
			handleSlashCommand(cmdName);
			return;
		}
	}

	if (text) {
		S.chatHistory.push(text);
		if (S.chatHistory.length > 200) S.setChatHistory(S.chatHistory.slice(-200));
		localStorage.setItem("moltis-chat-history", JSON.stringify(S.chatHistory));
	}
	S.setChatHistoryIdx(-1);
	S.setChatHistoryDraft("");
	S.chatInput.value = "";
	chatAutoResize();

	S.setChatSeq(S.chatSeq + 1);
	var msg = buildChatMessage(text, S.chatSeq);
	var chatParams = msg.params;
	var userEl = msg.el;

	var selectedModel = S.selectedModelId;
	if (selectedModel) {
		chatParams.model = selectedModel;
		setSessionModel(S.activeSessionKey, selectedModel);
	}
	bumpSessionCount(S.activeSessionKey, 1);
	setSessionReplying(S.activeSessionKey, true);
	sendRpc("chat.send", chatParams).then((res) => {
		if (res?.payload?.queued) {
			markMessageQueued(userEl, S.activeSessionKey);
		} else if (res && !res.ok && res.error) {
			chatAddMsg("error", res.error.message || "Request failed");
		}
	});
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
	'<div class="px-4 py-1.5 border-b border-[var(--border)] bg-[var(--surface)] flex items-center gap-2">' +
	'<div id="modelCombo" class="model-combo">' +
	'<button id="modelComboBtn" class="model-combo-btn" type="button">' +
	'<span id="modelComboLabel">loading\u2026</span>' +
	'<svg class="model-combo-chevron" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor" width="12" height="12"><path d="M19.5 8.25l-7.5 7.5-7.5-7.5"/></svg>' +
	"</button>" +
	'<div id="modelDropdown" class="model-dropdown hidden">' +
	'<input id="modelSearchInput" type="text" placeholder="Search models\u2026" class="model-search-input" autocomplete="off" />' +
	'<div id="modelDropdownList" class="model-dropdown-list"></div>' +
	"</div>" +
	"</div>" +
	'<button id="sandboxToggle" class="sandbox-toggle text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)]" style="display:inline-flex;align-items:center;gap:4px;" title="Toggle sandbox mode">' +
	'<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="14" height="14" style="flex-shrink:0;"><path d="M16.5 10.5V6.75a4.5 4.5 0 1 0-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 0 0 2.25-2.25v-6.75a2.25 2.25 0 0 0-2.25-2.25H6.75a2.25 2.25 0 0 0-2.25 2.25v6.75a2.25 2.25 0 0 0 2.25 2.25Z"/></svg>' +
	'<span id="sandboxLabel">sandboxed</span>' +
	"</button>" +
	'<div style="position:relative;display:inline-block">' +
	'<button id="sandboxImageBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)]" style="display:inline-flex;align-items:center;gap:4px;color:var(--muted);" title="Sandbox image">' +
	'<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="14" height="14" style="flex-shrink:0;"><path stroke-linecap="round" stroke-linejoin="round" d="m21 7.5-9-5.25L3 7.5m18 0-9 5.25m9-5.25v9l-9 5.25M3 7.5l9 5.25M3 7.5v9l9 5.25m0-9v9"/></svg>' +
	'<span id="sandboxImageLabel" style="max-width:120px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">ubuntu:25.10</span>' +
	"</button>" +
	'<div id="sandboxImageDropdown" class="hidden" style="position:absolute;top:100%;left:0;z-index:50;margin-top:4px;min-width:200px;max-height:300px;overflow-y:auto;background:var(--surface);border:1px solid var(--border);border-radius:8px;box-shadow:0 4px 12px rgba(0,0,0,.15);"></div>' +
	"</div>" +
	'<button id="mcpToggleBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)]" style="display:inline-flex;align-items:center;gap:4px;" title="Toggle MCP tools for this session">' +
	'<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="14" height="14" style="flex-shrink:0;"><path stroke-linecap="round" stroke-linejoin="round" d="M13.19 8.688a4.5 4.5 0 0 1 1.242 7.244l-4.5 4.5a4.5 4.5 0 0 1-6.364-6.364l1.757-1.757m13.35-.622 1.757-1.757a4.5 4.5 0 0 0-6.364-6.364l-4.5 4.5a4.5 4.5 0 0 0 1.242 7.244" /></svg>' +
	'<span id="mcpToggleLabel">MCP</span>' +
	"</button>" +
	'<button id="debugPanelBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)]" style="display:inline-flex;align-items:center;gap:4px;color:var(--muted);" title="Show context debug info">' +
	'<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="14" height="14" style="flex-shrink:0;"><path stroke-linecap="round" stroke-linejoin="round" d="M11.42 15.17 17.25 21A2.652 2.652 0 0 0 21 17.25l-5.877-5.877M11.42 15.17l2.496-3.03c.317-.384.74-.626 1.208-.766M11.42 15.17l-4.655 5.653a2.548 2.548 0 1 1-3.586-3.586l6.837-5.63m5.108-.233c.55-.164 1.163-.188 1.743-.14a4.5 4.5 0 0 0 4.486-6.336l-3.276 3.277a3.004 3.004 0 0 1-2.25-2.25l3.276-3.276a4.5 4.5 0 0 0-6.336 4.486c.049.58.025 1.194-.14 1.743" /></svg>' +
	'<span id="debugPanelLabel">Debug</span>' +
	"</button>" +
	'<button id="rawPromptBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)]" style="display:inline-flex;align-items:center;gap:4px;color:var(--muted);" title="Show raw system prompt sent to model">' +
	'<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="14" height="14" style="flex-shrink:0;"><path stroke-linecap="round" stroke-linejoin="round" d="M17.25 6.75 22.5 12l-5.25 5.25m-10.5 0L1.5 12l5.25-5.25m7.5-3-4.5 16.5" /></svg>' +
	'<span id="rawPromptLabel">Prompt</span>' +
	"</button>" +
	'<button id="fullContextBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)]" style="display:inline-flex;align-items:center;gap:4px;color:var(--muted);" title="Show full LLM context (system prompt + history)">' +
	'<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="14" height="14" style="flex-shrink:0;"><path stroke-linecap="round" stroke-linejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z" /></svg>' +
	'<span id="fullContextLabel">Context</span>' +
	"</button>" +
	'<div class="ml-auto flex items-center gap-1.5">' +
	'<span id="chatSessionName" class="text-xs text-[var(--muted)] cursor-default" title="Click to rename"></span>' +
	'<input id="chatSessionRenameInput" class="hidden text-xs text-[var(--text)] bg-[var(--surface2)] border border-[var(--border)] rounded-[var(--radius-sm)] px-1.5 py-0.5 outline-none max-w-[200px]" style="width:0" />' +
	'<button id="chatSessionFork" class="provider-btn provider-btn-secondary provider-btn-sm hidden">Fork</button>' +
	'<button id="chatSessionDelete" class="provider-btn provider-btn-danger provider-btn-sm hidden">Delete</button>' +
	"</div>" +
	"</div>" +
	"<div>" +
	'<div id="debugPanel" class="hidden px-4 py-3 border-b border-[var(--border)] bg-[var(--surface2)] overflow-y-auto" style="max-height:260px;"></div>' +
	'<div id="rawPromptPanel" class="hidden px-4 py-3 border-b border-[var(--border)] bg-[var(--surface2)] overflow-y-auto" style="max-height:400px;"></div>' +
	'<div id="fullContextPanel" class="hidden px-4 py-3 border-b border-[var(--border)] bg-[var(--surface2)] overflow-y-auto" style="max-height:500px;"></div>' +
	"</div>" +
	'<div class="p-4 flex flex-col gap-2" id="messages" style="overflow-y:auto;min-height:0"></div>' +
	'<div id="queuedMessages" class="queued-tray hidden"></div>' +
	'<div id="tokenBar" class="token-bar"></div>' +
	'<div class="px-4 py-3 border-t border-[var(--border)] bg-[var(--surface)] flex gap-2 items-end">' +
	'<textarea id="chatInput" placeholder="Type a message..." rows="1" ' +
	'class="flex-1 bg-[var(--surface2)] border border-[var(--border)] text-[var(--text)] px-3 py-2 rounded-lg text-sm resize-none min-h-[40px] max-h-[120px] leading-relaxed focus:outline-none focus:border-[var(--border-strong)] focus:ring-1 focus:ring-[var(--accent-subtle)] transition-colors font-[var(--font-body)]"></textarea>' +
	'<button id="micBtn" disabled title="Click to start recording" ' +
	'class="mic-btn min-h-[40px] px-3 bg-[var(--surface2)] border border-[var(--border)] rounded-lg text-[var(--muted)] cursor-pointer disabled:opacity-40 disabled:cursor-default transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text)]">' +
	'<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="18" height="18"><path stroke-linecap="round" stroke-linejoin="round" d="M12 18.75a6 6 0 0 0 6-6v-1.5m-6 7.5a6 6 0 0 1-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 0 1-3-3V4.5a3 3 0 1 1 6 0v8.25a3 3 0 0 1-3 3Z"/></svg>' +
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

registerPrefix(
	"/chats",
	function initChat(container, sessionKeyFromUrl) {
		container.style.cssText = "position:relative";
		// Safe: chatPageHTML is a static hardcoded template with no user input.
		// This is a compile-time constant defined above — no dynamic or user data.
		container.innerHTML = chatPageHTML; // eslint-disable-line no-unsanitized/property

		S.setChatMsgBox(S.$("messages"));
		S.setChatInput(S.$("chatInput"));
		S.setChatSendBtn(S.$("sendBtn"));

		S.setModelCombo(S.$("modelCombo"));
		S.setModelComboBtn(S.$("modelComboBtn"));
		S.setModelComboLabel(S.$("modelComboLabel"));
		S.setModelDropdown(S.$("modelDropdown"));
		S.setModelSearchInput(S.$("modelSearchInput"));
		S.setModelDropdownList(S.$("modelDropdownList"));
		bindModelComboEvents();

		S.setSandboxToggleBtn(S.$("sandboxToggle"));
		S.setSandboxLabel(S.$("sandboxLabel"));
		bindSandboxToggleEvents();
		updateSandboxUI(true);

		S.setSandboxImageBtn(S.$("sandboxImageBtn"));
		S.setSandboxImageLabel(S.$("sandboxImageLabel"));
		S.setSandboxImageDropdown(S.$("sandboxImageDropdown"));
		bindSandboxImageEvents();
		updateSandboxImageUI(null);
		updateChatSessionHeader();

		var mcpToggle = S.$("mcpToggleBtn");
		if (mcpToggle) mcpToggle.addEventListener("click", toggleMcp);
		updateMcpToggleUI(true); // default: MCP enabled

		var debugBtn = S.$("debugPanelBtn");
		if (debugBtn) debugBtn.addEventListener("click", toggleDebugPanel);

		var rawBtn = S.$("rawPromptBtn");
		if (rawBtn) rawBtn.addEventListener("click", toggleRawPromptPanel);

		S.$("fullContextBtn")?.addEventListener("click", toggleFullContextPanel);

		if (S.models.length > 0 && S.modelComboLabel) {
			var found = S.models.find((m) => m.id === S.selectedModelId);
			if (found) {
				S.modelComboLabel.textContent = found.displayName || found.id;
			} else if (S.models[0]) {
				S.modelComboLabel.textContent = S.models[0].displayName || S.models[0].id;
			}
		}

		// Determine session key from URL or localStorage
		var sessionKey;
		if (sessionKeyFromUrl) {
			sessionKey = sessionKeyFromUrl;
		} else {
			sessionKey = localStorage.getItem("moltis-session") || "main";
			history.replaceState(null, "", sessionPath(sessionKey));
		}

		if (S.connected) {
			S.chatSendBtn.disabled = false;
			switchSession(sessionKey);
		}

		S.chatInput.addEventListener("input", () => {
			chatAutoResize();
			slashHandleInput();
		});
		S.chatInput.addEventListener("keydown", (e) => {
			if (slashHandleKeydown(e)) return;
			if (e.key === "Enter" && !e.shiftKey) {
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
				return;
			}
		});
		S.chatSendBtn.addEventListener("click", sendChat);

		S.chatMsgBox.addEventListener("copy", handleChatCopy);

		// Initialize voice input
		initVoiceInput(S.$("micBtn"));

		// Initialize media drag-and-drop (the input area is the bottom bar)
		var inputArea = S.chatInput?.closest(".px-4.py-3");
		initMediaDrop(S.chatMsgBox, inputArea);

		S.chatInput.focus();
	},
	function teardownChat() {
		teardownVoiceInput();
		teardownMediaDrop();
		slashHideMenu();
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
		S.setSandboxToggleBtn(null);
		S.setSandboxLabel(null);
	},
);
