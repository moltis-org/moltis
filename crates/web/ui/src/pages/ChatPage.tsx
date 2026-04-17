// ── Chat page (Preact + JSX) ────────────────────────────────────────
// This is a TypeScript/JSX conversion of page-chat.js. The page is
// heavily imperative (DOM manipulation + registerPrefix router pattern)
// so the conversion preserves that style while adding types.

// NOTE: The chatPageHTML constant uses innerHTML assignment which is safe
// because it is a compile-time static string with no user input interpolated.
// The original JS file documents this explicitly. The eslint-disable comment
// is preserved from the original source.

import { effect } from "@preact/signals";
import { render } from "preact";
import { chatAddMsg, chatAddMsgWithImages, updateCommandInputUI } from "../chat-ui";
import { highlightCodeBlocks } from "../code-highlight";
import { SessionHeader } from "../components/session-header";
import { formatBytes, formatTokens, renderMarkdown, sendRpc, warmAudioPlayback } from "../helpers";
import {
	clearPendingImages,
	getPendingImages,
	hasPendingImages,
	initMediaDrop,
	teardownMediaDrop,
} from "../media-drop";
import { bindModelComboEvents, setSessionModel } from "../models";
import { bindNodeComboEvents, fetchNodes, unbindNodeEvents } from "../nodes-selector";
import { bindReasoningToggle, unbindReasoningToggle } from "../reasoning-toggle";
import { registerPrefix, sessionPath } from "../router";
import { routes } from "../routes";
import { bindSandboxImageEvents, bindSandboxToggleEvents, updateSandboxImageUI, updateSandboxUI } from "../sandbox";
import {
	bumpSessionCount,
	cacheOutgoingUserMessage,
	clearActiveSession,
	clearAllSessions,
	seedSessionPreviewFromUserText,
	setSessionActiveRunId,
	setSessionReplying,
	switchSession,
} from "../sessions";
import * as S from "../state";
import { modelStore } from "../stores/model-store";
import { sessionStore } from "../stores/session-store";
import { initVoiceInput, teardownVoiceInput } from "../voice-input";

// ── Types ───────────────────────────────────────────────────

interface SlashCommand {
	name: string;
	description: string;
}

interface ParsedSlash {
	name: string;
	args: string;
}

interface ContextData {
	session?: Record<string, unknown>;
	project?: Record<string, unknown> | null;
	tools?: Array<{ name: string; description?: string }>;
	skills?: Array<{ name: string; description?: string; source?: string }>;
	mcpServers?: Array<{ name: string; state?: string; tool_count?: number }>;
	mcpDisabled?: boolean;
	sandbox?: Record<string, unknown>;
	execution?: Record<string, unknown>;
	tokenUsage?: Record<string, number>;
	promptMemory?: PromptMemoryData | null;
	supportsTools?: boolean;
}

interface PromptMemoryData {
	mode?: string;
	present?: boolean;
	chars?: number;
	fileSource?: string;
	path?: string;
	snapshotActive?: boolean;
}

interface CompactCardData {
	mode?: string;
	messageCount?: number;
	totalTokens?: number;
	estimatedNextInputTokens?: number;
	contextWindow?: number;
	compactionTotalTokens?: number;
	compactionInputTokens?: number;
	compactionOutputTokens?: number;
	settingsHint?: string;
}

interface ModelNotice {
	id: string;
	displayName?: string;
	provider?: string;
	supportsTools?: boolean;
}

interface ContextMessage {
	role?: string;
	content?: unknown;
	tool_calls?: Array<{
		id?: string;
		function?: { name?: string; arguments?: string };
	}>;
	tool_call_id?: string;
}

// ── Slash commands ───────────────────────────────────────
const slashCommands: SlashCommand[] = [
	{ name: "clear", description: "Clear conversation history" },
	{ name: "compact", description: "Summarize conversation to save tokens" },
	{ name: "context", description: "Show session context and project info" },
	{ name: "sh", description: "Enter command mode (/sh off or Esc to exit)" },
];
let slashMenuEl: HTMLDivElement | null = null;
let slashMenuIdx = 0;
let slashMenuItems: SlashCommand[] = [];
let chatMoreModalKeydownHandler: ((e: KeyboardEvent) => void) | null = null;
let disposeSessionControlsVisibility: (() => void) | null = null;
let promptMemoryToolbarRequestId = 0;

function slashInjectStyles(): void {
	if (document.getElementById("slashMenuStyles")) return;
	const s = document.createElement("style");
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

// The file is extremely large (1773 lines of imperative DOM code).
// Due to the sheer size, the remaining implementation continues below
// with the same pattern: all `var` -> `const`/`let`, all `html\`\``
// tagged templates in render() calls -> JSX, all function params typed.
//
// For brevity in this conversion, the full imperative DOM manipulation
// functions (slash menu, context cards, debug panels, etc.) are preserved
// as-is with TypeScript annotations since they don't use HTM templates.

function slashShowMenu(filter: string): void {
	slashInjectStyles();
	const matches = slashCommands.filter((c) => `/${c.name}`.indexOf(filter) === 0);
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
		const item = document.createElement("div");
		item.className = `slash-menu-item${i === 0 ? " active" : ""}`;
		const nameSpan = document.createElement("span");
		nameSpan.className = "slash-name";
		nameSpan.textContent = `/${cmd.name}`;
		const descSpan = document.createElement("span");
		descSpan.className = "slash-desc";
		descSpan.textContent = cmd.description;
		item.appendChild(nameSpan);
		item.appendChild(descSpan);
		item.addEventListener("mousedown", (e: MouseEvent) => {
			e.preventDefault();
			slashSelectItem(i);
		});
		slashMenuEl!.appendChild(item);
	});

	const inputWrap = S.chatInput?.parentElement;
	if (inputWrap && !slashMenuEl.parentElement) {
		inputWrap.classList.add("relative");
		inputWrap.appendChild(slashMenuEl);
	}
}

function slashHideMenu(): void {
	if (slashMenuEl?.parentElement) {
		slashMenuEl.parentElement.removeChild(slashMenuEl);
	}
	slashMenuItems = [];
	slashMenuIdx = 0;
}

function slashSelectItem(idx: number): void {
	if (!slashMenuItems[idx]) return;
	S.chatInput.value = `/${slashMenuItems[idx].name}`;
	slashHideMenu();
	sendChat();
}

function slashHandleInput(): void {
	const val = S.chatInput.value;
	if (val.indexOf("/") === 0 && val.indexOf(" ") === -1) {
		slashShowMenu(val);
	} else {
		slashHideMenu();
	}
}

function slashHandleKeydown(e: KeyboardEvent): boolean {
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

function slashUpdateActive(): void {
	if (!slashMenuEl) return;
	const items = slashMenuEl.querySelectorAll(".slash-menu-item");
	items.forEach((el, i) => {
		el.classList.toggle("active", i === slashMenuIdx);
	});
}

function parseSlashCommand(text: string): ParsedSlash | null {
	if (!text || text.charAt(0) !== "/") return null;
	const body = text.substring(1).trim();
	if (!body) return null;
	const spaceIdx = body.indexOf(" ");
	if (spaceIdx === -1) return { name: body.toLowerCase(), args: "" };
	return {
		name: body.substring(0, spaceIdx).toLowerCase(),
		args: body.substring(spaceIdx + 1).trim(),
	};
}

function isShLocalToggle(args: string): boolean {
	if (!args) return true;
	const normalized = args.toLowerCase();
	return normalized === "on" || normalized === "off" || normalized === "exit";
}

function shouldHandleSlashLocally(cmdName: string, args: string): boolean {
	if (cmdName === "sh") return isShLocalToggle(args);
	return slashCommands.some((c) => c.name === cmdName);
}

function commandModeSummary(): string {
	const execModeLabel = S.sessionExecMode === "sandbox" ? "sandboxed" : "host";
	const promptSymbol = S.sessionExecPromptSymbol || "$";
	return `${execModeLabel}, prompt ${promptSymbol}`;
}

function setCommandMode(enabled: boolean): void {
	S.setCommandModeEnabled(!!enabled);
	updateCommandInputUI();
}

// ── Context card helpers ─────────────────────────────────
function ctxEl(tag: string, cls: string, text?: string): HTMLElement {
	const el = document.createElement(tag);
	if (cls) el.className = cls;
	if (text !== undefined) el.textContent = text;
	return el;
}

function ctxRow(label: string, value: string, mono?: boolean): HTMLElement {
	const row = ctxEl("div", "ctx-row");
	row.appendChild(ctxEl("span", "ctx-label", label));
	row.appendChild(ctxEl("span", `ctx-value${mono ? " mono" : ""}`, value));
	return row;
}

function ctxSection(title: string): HTMLElement {
	const sec = ctxEl("div", "ctx-section");
	sec.appendChild(ctxEl("div", "ctx-section-title", title));
	return sec;
}

function formatPromptMemoryMode(mode: string | undefined): string {
	if (mode === "frozen-at-session-start") return "Frozen at session start";
	if (mode === "live-reload") return "Live reload";
	return mode || "unknown";
}

function formatPromptMemorySource(source: string | undefined): string {
	if (source === "agent_workspace") return "Agent workspace";
	if (source === "root_workspace") return "Root workspace";
	return source || "unknown";
}

function buildPromptMemorySummary(promptMemory: PromptMemoryData | null): string {
	if (!promptMemory) return "Unavailable";
	const parts: string[] = [formatPromptMemoryMode(promptMemory.mode)];
	if (promptMemory.snapshotActive) parts.push("snapshot active");
	parts.push(promptMemory.present ? `${Number(promptMemory.chars || 0).toLocaleString()} chars` : "empty");
	return parts.join(" \u00b7 ");
}

function promptMemoryDetailParts(promptMemory: PromptMemoryData | null): string[] {
	if (!promptMemory) return [];
	const parts: string[] = [];
	if (promptMemory.fileSource) parts.push(`source ${formatPromptMemorySource(promptMemory.fileSource)}`);
	if (promptMemory.path) parts.push(promptMemory.path);
	return parts;
}

function promptMemoryToolbarTitle(promptMemory: PromptMemoryData | null): string {
	if (!promptMemory) return "Prompt memory unavailable";
	const parts = [`Prompt memory: ${buildPromptMemorySummary(promptMemory)}`];
	const dp = promptMemoryDetailParts(promptMemory);
	if (dp.length > 0) parts.push(dp.join(" \u00b7 "));
	return parts.join("\n");
}

function promptMemoryToolbarLabel(promptMemory: PromptMemoryData | null): string {
	if (!promptMemory) return "Memory";
	if (promptMemory.mode === "frozen-at-session-start") return "Memory frozen";
	if (promptMemory.mode === "live-reload") return "Memory live";
	return "Memory";
}

function setPromptMemoryToolbarState(pm: PromptMemoryData | null, loading: boolean, refreshing: boolean): void {
	const toolbar = S.$("promptMemoryToolbar") as HTMLElement | null;
	const statusBtn = S.$("promptMemoryStatusBtn") as HTMLButtonElement | null;
	const statusLabel = S.$("promptMemoryStatusLabel") as HTMLElement | null;
	const refreshBtn = S.$("promptMemoryRefreshBtn") as HTMLButtonElement | null;
	if (!(toolbar && statusBtn && statusLabel && refreshBtn)) return;
	toolbar.classList.remove("hidden");
	toolbar.classList.add("inline-flex");
	statusBtn.disabled = !!loading;
	refreshBtn.disabled = !!refreshing;
	if (loading) {
		statusLabel.textContent = "Memory\u2026";
		statusBtn.title = "Loading prompt memory status";
		refreshBtn.classList.add("hidden");
		return;
	}
	statusLabel.textContent = promptMemoryToolbarLabel(pm);
	statusBtn.title = promptMemoryToolbarTitle(pm);
	refreshBtn.classList.toggle("hidden", pm?.mode !== "frozen-at-session-start");
	refreshBtn.title = pm?.mode === "frozen-at-session-start" ? "Refresh frozen prompt memory" : "Refresh unavailable";
}

function refreshPromptMemoryToolbarFromPayload(pm: PromptMemoryData | null): void {
	setPromptMemoryToolbarState(pm || null, false, false);
}

function refreshPromptMemoryToolbar(): Promise<PromptMemoryData | null> {
	if (!S.connected) {
		setPromptMemoryToolbarState(null, false, false);
		return Promise.resolve(null);
	}
	const requestId = ++promptMemoryToolbarRequestId;
	setPromptMemoryToolbarState(null, true, false);
	return sendRpc("chat.context", {}).then((res: any) => {
		if (requestId !== promptMemoryToolbarRequestId) return null;
		if (res?.ok && res.payload) {
			const pm = res.payload.promptMemory || null;
			refreshPromptMemoryToolbarFromPayload(pm);
			return pm;
		}
		setPromptMemoryToolbarState(null, false, false);
		return null;
	});
}

function refreshPromptMemoryToolbarSnapshot(): Promise<PromptMemoryData | null> {
	setPromptMemoryToolbarState(null, false, true);
	return sendRpc("chat.prompt_memory.refresh", {})
		.then((res: any) => {
			if (!(res?.ok && res.payload)) throw new Error(res?.error?.message || "Failed to refresh prompt memory");
			const pm = res.payload.promptMemory || null;
			refreshPromptMemoryToolbarFromPayload(pm);
			maybeRefreshFullContext();
			return pm;
		})
		.catch((error: any) => {
			refreshPromptMemoryToolbar();
			chatAddMsg("error", error?.message || "Failed to refresh prompt memory");
			return null;
		});
}

// ── Context card section renderers ───────────────────────
function renderContextSessionSection(card: HTMLElement, data: ContextData): void {
	const sess: any = data.session || {};
	const sec = ctxSection("Session");
	sec.appendChild(ctxRow("Key", sess.key || "unknown", true));
	sec.appendChild(ctxRow("Messages", String(sess.messageCount || 0)));
	sec.appendChild(ctxRow("Model", sess.model || "default", true));
	if (sess.provider) sec.appendChild(ctxRow("Provider", sess.provider, true));
	if (sess.label) sec.appendChild(ctxRow("Label", sess.label));
	sec.appendChild(ctxRow("Tool Support", data.supportsTools === false ? "Disabled" : "Enabled"));
	card.appendChild(sec);
}

function renderContextProjectSection(card: HTMLElement, data: ContextData): void {
	const proj: any = data.project;
	const sec = ctxSection("Project");
	if (proj) {
		sec.appendChild(ctxRow("Name", proj.label || "(unnamed)"));
		if (proj.directory) sec.appendChild(ctxRow("Directory", proj.directory, true));
		if (proj.systemPrompt) sec.appendChild(ctxRow("System Prompt", `${proj.systemPrompt.length} chars`));
		const ctxFiles: any[] = proj.contextFiles || [];
		if (ctxFiles.length > 0) {
			const fl = ctxEl("div", "ctx-section-title", `Context Files (${ctxFiles.length})`);
			fl.classList.add("spaced");
			sec.appendChild(fl);
			ctxFiles.forEach((f: any) => {
				const row = ctxEl("div", "ctx-file");
				row.appendChild(ctxEl("span", "ctx-file-path", f.path));
				row.appendChild(ctxEl("span", "ctx-file-size", formatBytes(f.size)));
				sec.appendChild(row);
			});
		}
	} else {
		sec.appendChild(ctxEl("div", "ctx-empty", "No project bound to this session"));
	}
	card.appendChild(sec);
}

function renderContextToolsSection(card: HTMLElement, data: ContextData): void {
	const tools = data.tools || [];
	const sec = ctxSection("Tools");
	if (data.supportsTools === false) {
		sec.appendChild(ctxEl("div", "ctx-disabled", "Tools disabled \u2014 model doesn't support tool calling"));
	} else if (tools.length > 0) {
		const wrap = ctxEl("div", "ctx-tool-wrap");
		tools.forEach((t) => {
			const tag = ctxEl("span", "ctx-tag");
			tag.appendChild(ctxEl("span", "ctx-tag-dot"));
			tag.appendChild(document.createTextNode(t.name));
			tag.title = t.description || "";
			wrap.appendChild(tag);
		});
		sec.appendChild(wrap);
	} else {
		sec.appendChild(ctxEl("div", "ctx-empty", "No tools registered"));
	}
	card.appendChild(sec);
}

function renderContextSkillsSection(card: HTMLElement, data: ContextData): void {
	const skills = data.skills || [];
	const sec = ctxSection("Skills & Plugins");
	if (data.supportsTools === false) {
		sec.appendChild(ctxEl("div", "ctx-disabled", "Skills disabled \u2014 model doesn't support tool calling"));
	} else if (skills.length > 0) {
		const wrap = ctxEl("div", "ctx-tool-wrap");
		skills.forEach((s) => {
			const tag = ctxEl("span", "ctx-tag");
			const dot = ctxEl("span", "ctx-tag-dot");
			const isPlugin = s.source === "plugin";
			(dot as HTMLElement).style.background = isPlugin ? "var(--accent)" : "var(--success, #4a9)";
			tag.appendChild(dot);
			tag.appendChild(document.createTextNode(s.name));
			tag.title = (isPlugin ? "[Plugin] " : "[Skill] ") + (s.description || "");
			wrap.appendChild(tag);
		});
		sec.appendChild(wrap);
	} else {
		sec.appendChild(ctxEl("div", "ctx-empty", "No skills or plugins enabled"));
	}
	card.appendChild(sec);
}

function renderContextMcpSection(card: HTMLElement, data: ContextData): void {
	const servers = data.mcpServers || [];
	const sec = ctxSection("MCP Tools");
	if (data.supportsTools === false) {
		sec.appendChild(ctxEl("div", "ctx-disabled", "MCP tools disabled \u2014 model doesn't support tool calling"));
	} else if (data.mcpDisabled) {
		sec.appendChild(ctxEl("div", "ctx-disabled", "MCP tools disabled for this session"));
	} else {
		const running = servers.filter((s) => s.state === "running");
		if (running.length > 0) {
			const wrap = ctxEl("div", "ctx-tool-wrap");
			running.forEach((s) => {
				const tag = ctxEl("span", "ctx-tag");
				const dot = ctxEl("span", "ctx-tag-dot");
				(dot as HTMLElement).style.background = "var(--ok)";
				tag.appendChild(dot);
				tag.appendChild(document.createTextNode(s.name));
				tag.title = `${s.tool_count} tool${s.tool_count !== 1 ? "s" : ""} \u2014 ${s.state}`;
				wrap.appendChild(tag);
			});
			sec.appendChild(wrap);
		} else {
			sec.appendChild(ctxEl("div", "ctx-empty", "No MCP tools running"));
		}
	}
	card.appendChild(sec);
}

function renderContextSandboxSection(card: HTMLElement, data: ContextData): void {
	const sb: any = data.sandbox || {};
	const exec: any = data.execution || {};
	const sec = ctxSection("Sandbox");
	sec.appendChild(ctxRow("Enabled", sb.enabled ? "yes" : "no", true));
	let execLabel = exec.mode ? (exec.mode === "sandbox" ? "sandboxed" : "host") : "";
	if (execLabel && exec.promptSymbol) execLabel += ` (${exec.promptSymbol})`;
	if (execLabel) sec.appendChild(ctxRow("Command route", execLabel, true));
	for (const [label, value, mono] of [
		["Backend", sb.backend, false],
		["Mode", sb.mode, false],
		["Scope", sb.scope, false],
		["Workspace Mount", sb.workspaceMount, false],
		["Image", sb.image, true],
		["Container", sb.containerName, false],
	] as [string, string, boolean][]) {
		if (value) sec.appendChild(ctxRow(label, value, mono));
	}
	card.appendChild(sec);
}

function renderContextTokensSection(card: HTMLElement, data: ContextData): void {
	const tu: any = data.tokenUsage || {};
	const sessionInput = tu.inputTokens || 0;
	const sessionOutput = tu.outputTokens || 0;
	const sessionCacheRead = tu.cacheReadTokens || 0;
	const sessionCacheWrite = tu.cacheWriteTokens || 0;
	const sessionTotal = tu.total || 0;
	const currentInput = tu.currentInputTokens || sessionInput;
	const currentOutput = tu.currentOutputTokens || 0;
	const currentCacheRead = tu.currentCacheReadTokens || 0;
	const currentCacheWrite = tu.currentCacheWriteTokens || 0;
	const currentTotal = tu.currentTotal || currentInput + currentOutput;
	const estimatedNextInput = tu.estimatedNextInputTokens || currentInput;
	const sec = ctxSection("Token Usage");
	sec.appendChild(ctxRow("Session input", formatTokens(sessionInput), true));
	sec.appendChild(ctxRow("Session output", formatTokens(sessionOutput), true));
	if (sessionCacheRead > 0) sec.appendChild(ctxRow("Session cached input", formatTokens(sessionCacheRead), true));
	if (sessionCacheWrite > 0) sec.appendChild(ctxRow("Session cache writes", formatTokens(sessionCacheWrite), true));
	sec.appendChild(ctxRow("Session total", formatTokens(sessionTotal), true));
	sec.appendChild(ctxRow("Current input", formatTokens(currentInput), true));
	sec.appendChild(ctxRow("Current output", formatTokens(currentOutput), true));
	if (currentCacheRead > 0) sec.appendChild(ctxRow("Current cached input", formatTokens(currentCacheRead), true));
	if (currentCacheWrite > 0) sec.appendChild(ctxRow("Current cache writes", formatTokens(currentCacheWrite), true));
	sec.appendChild(ctxRow("Current total", formatTokens(currentTotal), true));
	sec.appendChild(ctxRow("Estimated next input", formatTokens(estimatedNextInput), true));
	if (tu.contextWindow > 0) {
		const pct = Math.max(0, 100 - Math.round((estimatedNextInput / tu.contextWindow) * 100));
		sec.appendChild(ctxRow("Context left", `${pct}% of ${formatTokens(tu.contextWindow)}`, true));
	}
	card.appendChild(sec);
}

function renderContextPromptMemorySection(card: HTMLElement, data: ContextData): void {
	const pm = data.promptMemory || null;
	const sec = ctxSection("Prompt Memory");
	sec.appendChild(ctxRow("Status", buildPromptMemorySummary(pm)));
	if (pm) {
		sec.appendChild(ctxRow("Mode", formatPromptMemoryMode(pm.mode)));
		sec.appendChild(ctxRow("Present", pm.present ? "yes" : "no"));
		sec.appendChild(ctxRow("Chars", Number(pm.chars || 0).toLocaleString(), true));
		if (pm.fileSource) sec.appendChild(ctxRow("Source", formatPromptMemorySource(pm.fileSource)));
		if (pm.path) sec.appendChild(ctxRow("Path", pm.path, true));
	}
	card.appendChild(sec);
}

function renderContextCard(data: ContextData): void {
	if (!S.chatMsgBox) return;
	slashInjectStyles();
	const card = ctxEl("div", "ctx-card");
	const header = ctxEl("div", "ctx-header");
	const icon = document.createElement("span");
	icon.className = "icon icon-settings-gear";
	header.appendChild(icon);
	header.appendChild(ctxEl("span", "ctx-header-title", "Context"));
	card.appendChild(header);
	if (data.supportsTools === false) {
		const warning = ctxEl("div", "ctx-warning");
		const warnIcon = document.createElement("span");
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

const COMPACTION_MODE_LABELS: Record<string, string> = {
	deterministic: "Deterministic",
	recency_preserving: "Recency preserving",
	structured: "Structured",
	llm_replace: "LLM replace",
};

function compactionModeLabel(mode: string | undefined): string {
	if (!mode) return "Unknown";
	return COMPACTION_MODE_LABELS[mode] || mode;
}

export function renderCompactCard(data: CompactCardData): void {
	if (!S.chatMsgBox) return;
	slashInjectStyles();
	const card = ctxEl("div", "ctx-card");
	const header = ctxEl("div", "ctx-header");
	const icon = document.createElement("span");
	icon.className = "icon icon-compress";
	header.appendChild(icon);
	header.appendChild(ctxEl("span", "ctx-header-title", "Conversation compacted"));
	card.appendChild(header);
	if (data.mode) {
		const stratSec = ctxSection("Strategy");
		stratSec.appendChild(ctxRow("Mode", compactionModeLabel(data.mode)));
		const totalTokens = Number(data.compactionTotalTokens || 0);
		if (totalTokens > 0) {
			const inp = Number(data.compactionInputTokens || 0);
			const outp = Number(data.compactionOutputTokens || 0);
			stratSec.appendChild(
				ctxRow("Tokens used", `${formatTokens(totalTokens)} (${formatTokens(inp)} in + ${formatTokens(outp)} out)`),
			);
		} else {
			stratSec.appendChild(ctxRow("Tokens used", "0 (no LLM call)"));
		}
		card.appendChild(stratSec);
	}
	const statsSec = ctxSection("Before compact");
	statsSec.appendChild(ctxRow("Messages", String(data.messageCount || 0)));
	if (data.totalTokens) statsSec.appendChild(ctxRow("Total tokens", formatTokens(data.totalTokens || 0)));
	if (data.estimatedNextInputTokens)
		statsSec.appendChild(ctxRow("Estimated next input", formatTokens(data.estimatedNextInputTokens), true));
	if (data.contextWindow) {
		const basis = data.estimatedNextInputTokens || data.totalTokens || 0;
		const pctUsed = Math.round((basis / data.contextWindow) * 100);
		statsSec.appendChild(ctxRow("Context usage", `${pctUsed}% of ${formatTokens(data.contextWindow)}`));
	}
	card.appendChild(statsSec);
	const afterSec = ctxSection("After compact");
	const replacesAll = data.mode === "deterministic" || data.mode === "llm_replace" || !data.mode;
	if (replacesAll) {
		afterSec.appendChild(ctxRow("Messages", "1 (summary)"));
		afterSec.appendChild(ctxRow("Status", "Conversation history replaced with a summary"));
	} else {
		afterSec.appendChild(ctxRow("Status", "Head + tail preserved verbatim; middle summarised"));
	}
	card.appendChild(afterSec);
	if (data.settingsHint) {
		const hintSec = ctxSection("Configure");
		const hintRow = ctxEl("div", "ctx-value");
		hintRow.textContent = data.settingsHint;
		hintSec.appendChild(hintRow);
		card.appendChild(hintSec);
	}
	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

// ── Debug / full context panels ──────────────────────────
function setDebugModalOpen(open: boolean): void {
	const modal = S.$("debugModal") as HTMLElement | null;
	if (!modal) return;
	modal.classList.toggle("hidden", !open);
	const btn = S.$("debugPanelBtn") as HTMLElement | null;
	if (btn) btn.style.color = open ? "var(--accent)" : "var(--muted)";
}

function setFullContextModalOpen(open: boolean): void {
	const modal = S.$("fullContextModal") as HTMLElement | null;
	if (!modal) return;
	modal.classList.toggle("hidden", !open);
	const btn = S.$("fullContextBtn") as HTMLElement | null;
	if (btn) btn.style.color = open ? "var(--accent)" : "var(--muted)";
}

function refreshDebugPanel(): void {
	const panel = S.$("debugPanel") as HTMLElement | null;
	if (!panel) return;
	panel.textContent = "";
	panel.appendChild(ctxEl("div", "text-xs text-[var(--muted)]", "Loading context\u2026"));
	sendRpc("chat.context", {}).then((res: any) => {
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

function toggleDebugPanel(): void {
	const modal = S.$("debugModal") as HTMLElement | null;
	if (!modal) return;
	const opening = modal.classList.contains("hidden");
	if (!opening) { setDebugModalOpen(false); return; }
	setFullContextModalOpen(false);
	setDebugModalOpen(true);
	refreshDebugPanel();
}

// ── Full context panel ───────────────────────────────────
const ROLE_COLORS: Record<string, string> = {
	system: "var(--accent)", user: "var(--ok, #22c55e)",
	assistant: "var(--info, #3b82f6)", tool: "var(--muted)",
};

function ctxMsgBadge(role: string): HTMLElement {
	const color = ROLE_COLORS[role] || "var(--text)";
	const badge = ctxEl("span", "text-xs font-semibold uppercase px-1.5 py-0.5 rounded");
	badge.style.cssText = `color:${color};background:color-mix(in srgb, ${color} 15%, transparent)`;
	badge.textContent = role;
	return badge;
}

function ctxMsgMeta(msg: ContextMessage, contentStr: string): string {
	const parts: string[] = [];
	const chars = contentStr ? contentStr.length : 0;
	if (chars > 0) parts.push(`${chars.toLocaleString()} chars`);
	const toolCalls = msg.tool_calls || [];
	if (toolCalls.length > 0) parts.push(`${toolCalls.length} tool call${toolCalls.length > 1 ? "s" : ""}`);
	if (msg.role === "tool" && msg.tool_call_id) parts.push(`id: ${msg.tool_call_id}`);
	return parts.join(" \u00b7 ");
}

function ctxMsgToolCall(tc: NonNullable<ContextMessage["tool_calls"]>[number]): HTMLElement {
	const div = ctxEl("div", "mt-1 border border-[var(--border)] rounded-md p-2 bg-[var(--surface)]");
	const hdr = ctxEl("div", "text-xs font-semibold text-[var(--text)] mb-1");
	hdr.textContent = `\ud83d\udee0 ${tc.function?.name || "unknown"}`;
	if (tc.id) hdr.appendChild(ctxEl("span", "font-normal text-[var(--muted)] ml-2", `id: ${tc.id}`));
	div.appendChild(hdr);
	if (tc.function?.arguments) {
		const pre = ctxEl("pre", "text-xs font-mono whitespace-pre-wrap break-words text-[var(--text)]");
		try { pre.textContent = JSON.stringify(JSON.parse(tc.function.arguments), null, 2); }
		catch { pre.textContent = tc.function.arguments; }
		div.appendChild(pre);
	}
	return div;
}

function renderContextMessage(msg: ContextMessage, index: number): HTMLElement {
	const wrapper = ctxEl("div", "mb-2");
	const contentStr = typeof msg.content === "string" ? msg.content : JSON.stringify(msg.content, null, 2);
	const hdr = ctxEl("div", "flex items-center gap-2 cursor-pointer select-none");
	hdr.appendChild(ctxMsgBadge(msg.role || "unknown"));
	hdr.appendChild(ctxEl("span", "text-xs text-[var(--muted)]", `#${index}`));
	const meta = ctxMsgMeta(msg, contentStr);
	if (meta) hdr.appendChild(ctxEl("span", "text-xs text-[var(--muted)]", meta));
	const chevron = ctxEl("span", "text-xs text-[var(--muted)] ml-auto");
	const startOpen = index !== 0;
	chevron.textContent = startOpen ? "\u25bc" : "\u25b6";
	hdr.appendChild(chevron);
	wrapper.appendChild(hdr);
	const body = ctxEl("div", "mt-1");
	body.style.display = startOpen ? "block" : "none";
	hdr.addEventListener("click", () => {
		const open = body.style.display !== "none";
		body.style.display = open ? "none" : "block";
		chevron.textContent = open ? "\u25b6" : "\u25bc";
	});
	if (contentStr) {
		const pre = ctxEl("pre", "text-xs font-mono whitespace-pre-wrap break-words bg-[var(--surface)] border border-[var(--border)] rounded-md p-2 text-[var(--text)]");
		pre.textContent = contentStr;
		body.appendChild(pre);
	}
	for (const tc of msg.tool_calls || []) body.appendChild(ctxMsgToolCall(tc));
	wrapper.appendChild(body);
	return wrapper;
}

function buildFullContextPromptMemoryBox(pm: PromptMemoryData | null): HTMLElement | null {
	if (!pm) return null;
	const box = ctxEl("div", "text-xs mb-3 rounded-md border border-[var(--border)] bg-[var(--surface)] p-2 text-[var(--text)]");
	const summaryLine = ctxEl("div", "font-semibold");
	summaryLine.textContent = `Prompt memory: ${buildPromptMemorySummary(pm)}`;
	box.appendChild(summaryLine);
	const dp = promptMemoryDetailParts(pm);
	if (dp.length > 0) box.appendChild(ctxEl("div", "mt-1 text-[var(--muted)]", dp.join(" \u00b7 ")));
	return box;
}

function appendFullContextWorkspaceWarnings(panel: HTMLElement, payload: any): void {
	const wf: any[] = Array.isArray(payload.workspaceFiles) ? payload.workspaceFiles : [];
	if (!payload.truncated || wf.length === 0) return;
	const tf = wf.filter((f: any) => f?.truncated);
	if (tf.length === 0) return;
	const warning = ctxEl("div", "text-xs mb-3 rounded-md border border-[var(--border)] bg-[var(--surface)] p-2 text-[var(--text)]");
	warning.textContent = tf.map((f: any) => {
		const name = typeof f.name === "string" ? f.name : "workspace file";
		return `${name}: ${Number(f.original_chars || 0).toLocaleString()} chars, limit ${Number(f.limit_chars || 0).toLocaleString()}, truncated by ${Number(f.truncated_chars || 0).toLocaleString()}`;
	}).join(" | ");
	panel.appendChild(warning);
}

function buildFullContextHeaderRow(payload: any, onRefresh: (btn: HTMLButtonElement) => void): { headerRow: HTMLElement; copyBtn: HTMLElement; downloadBtn: HTMLElement; llmOutputBtn: HTMLElement } {
	const headerRow = ctxEl("div", "flex items-center gap-3 mb-3");
	const headerText = ctxEl("span", "text-xs text-[var(--muted)]");
	headerText.textContent = `${payload.messageCount} messages \u00b7 system prompt ${payload.systemPromptChars.toLocaleString()} chars \u00b7 total ${payload.totalChars.toLocaleString()} chars`;
	headerRow.appendChild(headerText);
	const copyBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm"); copyBtn.textContent = "Copy";
	const downloadBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm"); downloadBtn.textContent = "Download";
	const llmOutputBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm"); llmOutputBtn.textContent = "LLM output";
	headerRow.appendChild(copyBtn); headerRow.appendChild(downloadBtn); headerRow.appendChild(llmOutputBtn);
	const pm = payload.promptMemory || null;
	if (pm?.mode === "frozen-at-session-start") {
		const rb = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm") as HTMLButtonElement;
		rb.textContent = "Refresh memory";
		rb.addEventListener("click", () => onRefresh(rb));
		headerRow.appendChild(rb);
	}
	return { headerRow, copyBtn, downloadBtn, llmOutputBtn };
}

function wireFullContextCopyButton(copyBtn: HTMLElement, messages: ContextMessage[], llmOutputs: any[], llmOutputPanel: HTMLElement): void {
	copyBtn.addEventListener("click", () => {
		const lines = messages.map((m) => {
			const content = typeof m.content === "string" ? m.content : JSON.stringify(m.content);
			const parts = [content];
			for (const tc of m.tool_calls || []) parts.push(`[tool_call: ${tc.function?.name || "?"} ${tc.function?.arguments || ""}]`);
			return `[${m.role}] ${parts.join("\n")}`;
		});
		const contextText = lines.join("\n");
		let copyText = contextText;
		const llmOutputVisible = llmOutputPanel && !llmOutputPanel.classList.contains("hidden");
		if (llmOutputVisible) copyText = `LLM output:\n${JSON.stringify(llmOutputs, null, 2)}\n\nContext:\n${contextText}`;
		navigator.clipboard.writeText(copyText).then(() => { copyBtn.textContent = "Copied!"; setTimeout(() => { copyBtn.textContent = "Copy"; }, 1500); });
	});
}

function wireFullContextDownloadButton(downloadBtn: HTMLElement, messages: ContextMessage[]): void {
	downloadBtn.addEventListener("click", () => {
		const lines = messages.map((m) => JSON.stringify(m));
		const blob = new Blob([`${lines.join("\n")}\n`], { type: "application/x-jsonlines" });
		const url = URL.createObjectURL(blob);
		const a = document.createElement("a");
		a.href = url;
		a.download = `context-${new Date().toISOString().slice(0, 19).replace(/[T:]/g, "-")}.jsonl`;
		a.click();
		URL.revokeObjectURL(url);
	});
}

function buildFullContextLlmOutputPanel(llmOutputs: any[]): HTMLElement {
	const panel = ctxEl("div", "hidden mb-3");
	panel.appendChild(ctxEl("div", "text-xs text-[var(--muted)] mb-1", `${llmOutputs.length} assistant output${llmOutputs.length === 1 ? "" : "s"}`));
	const pre = ctxEl("pre", "text-xs font-mono whitespace-pre-wrap break-words bg-[var(--surface)] border border-[var(--border)] rounded-md p-2 text-[var(--text)]");
	pre.id = "fullContextLlmOutput";
	pre.textContent = JSON.stringify(llmOutputs, null, 2);
	panel.appendChild(pre);
	return panel;
}

function wireFullContextLlmOutputToggle(button: HTMLElement, panel: HTMLElement): void {
	button.addEventListener("click", () => {
		const hidden = panel.classList.contains("hidden");
		panel.classList.toggle("hidden", !hidden);
		button.textContent = hidden ? "Hide LLM output" : "LLM output";
	});
}

function refreshFullContextMemory(refreshBtn: HTMLButtonElement): void {
	refreshBtn.disabled = true; refreshBtn.textContent = "Refreshing\u2026";
	refreshPromptMemoryToolbarSnapshot().then(() => { refreshBtn.disabled = false; refreshBtn.textContent = "Refresh memory"; });
}

function refreshFullContextPanel(): void {
	const panel = S.$("fullContextPanel") as HTMLElement | null;
	if (!panel) return;
	panel.textContent = "";
	panel.appendChild(ctxEl("div", "text-xs text-[var(--muted)]", "Building full context\u2026"));
	sendRpc("chat.full_context", {}).then((res: any) => {
		panel.textContent = "";
		if (!(res?.ok && res.payload)) { panel.appendChild(ctxEl("div", "text-xs text-[var(--error)]", "Failed to build context")); return; }
		const pm = res.payload.promptMemory || null;
		refreshPromptMemoryToolbarFromPayload(pm);
		const pmBox = buildFullContextPromptMemoryBox(pm);
		if (pmBox) panel.appendChild(pmBox);
		appendFullContextWorkspaceWarnings(panel, res.payload);
		const messages: ContextMessage[] = res.payload.messages || [];
		const llmOutputs = res.payload.llmOutputs || [];
		const llmOutputPanel = buildFullContextLlmOutputPanel(llmOutputs);
		const header = buildFullContextHeaderRow(res.payload, refreshFullContextMemory);
		wireFullContextCopyButton(header.copyBtn, messages, llmOutputs, llmOutputPanel);
		wireFullContextDownloadButton(header.downloadBtn, messages);
		wireFullContextLlmOutputToggle(header.llmOutputBtn, llmOutputPanel);
		panel.appendChild(header.headerRow);
		panel.appendChild(llmOutputPanel);
		for (let i = 0; i < messages.length; i++) panel.appendChild(renderContextMessage(messages[i], i));
	});
}

function toggleFullContextPanel(): void {
	const modal = S.$("fullContextModal") as HTMLElement | null;
	if (!modal) return;
	const opening = modal.classList.contains("hidden");
	if (!opening) { setFullContextModalOpen(false); return; }
	setDebugModalOpen(false);
	setFullContextModalOpen(true);
	refreshFullContextPanel();
}

export function maybeRefreshFullContext(): void {
	const modal = S.$("fullContextModal") as HTMLElement | null;
	if (modal && !modal.classList.contains("hidden")) refreshFullContextPanel();
}

// ── MCP toggle ───────────────────────────────────────────
export function updateMcpToggleUI(enabled: boolean): void {
	const btn = S.$("mcpToggleBtn") as HTMLElement | null;
	const label = S.$("mcpToggleLabel") as HTMLElement | null;
	if (!btn) return;
	if (enabled) {
		btn.style.color = "var(--ok)"; btn.style.borderColor = "var(--ok)";
		if (label) label.textContent = "MCP";
		btn.title = "MCP tools enabled \u2014 click to disable for this session";
	} else {
		btn.style.color = "var(--muted)"; btn.style.borderColor = "var(--border)";
		if (label) label.textContent = "MCP off";
		btn.title = "MCP tools disabled \u2014 click to enable for this session";
	}
}

function toggleMcp(): void {
	const label = S.$("mcpToggleLabel") as HTMLElement | null;
	const isEnabled = label && label.textContent === "MCP";
	const newDisabled = isEnabled;
	sendRpc("sessions.patch", { key: S.activeSessionKey, mcpDisabled: newDisabled }).then((res: any) => {
		if (res?.ok) updateMcpToggleUI(!newDisabled);
	});
}

export function showModelNotice(model: ModelNotice): void {
	if (!S.chatMsgBox) return;
	if (model.supportsTools !== false) return;
	slashInjectStyles();
	const tpl = document.getElementById("tpl-model-notice") as HTMLTemplateElement | null;
	if (!tpl) return;
	const card = (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
	const nameEl = card.querySelector("[data-model-name]");
	if (nameEl) nameEl.textContent = model.displayName || model.id;
	const providerEl = card.querySelector("[data-provider]");
	if (providerEl) providerEl.textContent = model.provider || "local";
	S.chatMsgBox.appendChild(card);
	S.chatMsgBox.scrollTop = S.chatMsgBox.scrollHeight;
}

// ── Slash command handlers ───────────────────────────────
function handleSlashCommand(cmdName: string, cmdArgs: string): void {
	if (cmdName === "clear") { clearActiveSession(); return; }
	if (cmdName === "compact") {
		chatAddMsg("system", "Compacting conversation\u2026");
		sendRpc("chat.compact", {}).then((res: any) => {
			if (res?.ok) switchSession(S.activeSessionKey);
			else chatAddMsg("error", res?.error?.message || "Compact failed");
		});
		return;
	}
	if (cmdName === "context") {
		chatAddMsg("system", "Loading context\u2026");
		sendRpc("chat.context", {}).then((res: any) => {
			if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
			if (res?.ok && res.payload) {
				try { renderContextCard(res.payload); }
				catch (err: any) { chatAddMsg("error", `Render error: ${err.message}`); }
			} else chatAddMsg("error", res?.error?.message || "Context failed");
		});
		return;
	}
	if (cmdName === "sh") {
		const normalized = (cmdArgs || "").toLowerCase();
		if (normalized === "off" || normalized === "exit") {
			setCommandMode(false);
			chatAddMsg("system", renderMarkdown("**Command:** mode disabled"), true);
			return;
		}
		setCommandMode(true);
		chatAddMsg("system", renderMarkdown(`**Command:** mode enabled (${commandModeSummary()}) \u00b7 exit with /sh off or Esc`), true);
	}
}

function tryHandleLocalSlashCommand(text: string, hasImages: boolean): boolean {
	if (text.charAt(0) !== "/" || hasImages) return false;
	const slash = parseSlashCommand(text);
	if (!(slash && shouldHandleSlashLocally(slash.name, slash.args))) return false;
	S.chatInput.value = "";
	chatAutoResize();
	slashHideMenu();
	handleSlashCommand(slash.name, slash.args);
	return true;
}

function rememberChatHistory(text: string): void {
	if (!text) return;
	S.chatHistory.push(text);
	if (S.chatHistory.length > 200) S.setChatHistory(S.chatHistory.slice(-200));
	localStorage.setItem("moltis-chat-history", JSON.stringify(S.chatHistory));
}

function resetComposerAfterSend(): void {
	S.setChatHistoryIdx(-1); S.setChatHistoryDraft(""); S.chatInput.value = "";
	chatAutoResize();
	if (window.innerWidth < 768) S.chatInput.blur();
}

function normalizeOutgoingText(text: string, hasImages: boolean): string {
	if (!(S.commandModeEnabled && text && !hasImages)) return text;
	const parsed = parseSlashCommand(text);
	if (parsed && parsed.name === "sh") return text;
	return `/sh ${text}`;
}

function applySelectedModelToChatParams(chatParams: Record<string, unknown>): void {
	const effectiveId = modelStore.effectiveModelId.value;
	if (!effectiveId) return;
	chatParams.model = effectiveId;
	setSessionModel(S.activeSessionKey, effectiveId);
}

function handleChatSendRpcResponse(res: any, userEl: HTMLElement | null): void {
	if (res?.ok && res.payload?.runId) setSessionActiveRunId(S.activeSessionKey, res.payload.runId);
	if (res?.payload?.queued) { markMessageQueued(userEl, S.activeSessionKey); return; }
	if (res && !res.ok && res.error) chatAddMsg("error", res.error.message || "Request failed");
}

function buildChatMessage(text: string, seq: number, displayText?: string): { params: Record<string, unknown>; el: HTMLElement | null } {
	const userText = displayText !== undefined ? displayText : text;
	const images = hasPendingImages() ? getPendingImages() : [];
	if (images.length > 0) {
		const content: Array<Record<string, unknown>> = [];
		if (text) content.push({ type: "text", text });
		for (const img of images) content.push({ type: "image_url", image_url: { url: (img as any).dataUrl } });
		const params = { content, _seq: seq };
		const el = chatAddMsgWithImages("user", userText ? renderMarkdown(userText) : "", images);
		clearPendingImages();
		return { params, el };
	}
	return { params: { text, _seq: seq }, el: chatAddMsg("user", renderMarkdown(userText), true) };
}

function sendChat(): void {
	const text = S.chatInput.value.trim();
	const hasImages = hasPendingImages();
	if (!((text || hasImages) && S.connected)) return;
	warmAudioPlayback();
	if (tryHandleLocalSlashCommand(text, hasImages)) return;
	rememberChatHistory(text);
	resetComposerAfterSend();
	const outgoingText = normalizeOutgoingText(text, hasImages);
	S.setChatSeq(S.chatSeq + 1);
	const msg = buildChatMessage(outgoingText, S.chatSeq, text);
	const chatParams = msg.params;
	const userEl = msg.el;
	if (userEl) highlightCodeBlocks(userEl);
	applySelectedModelToChatParams(chatParams);
	bumpSessionCount(S.activeSessionKey, 1);
	cacheOutgoingUserMessage(S.activeSessionKey, chatParams);
	seedSessionPreviewFromUserText(S.activeSessionKey, text || outgoingText);
	setSessionReplying(S.activeSessionKey, true);
	sendRpc("chat.send", chatParams).then((res: any) => handleChatSendRpcResponse(res, userEl));
	maybeRefreshFullContext();
}

function markMessageQueued(el: HTMLElement | null, sessionKey: string): void {
	if (!el) return;
	const tray = document.getElementById("queuedMessages");
	if (!tray) return;
	console.debug("[queued] marking user message as queued, moving to tray", { sessionKey });
	el.classList.add("queued");
	const badge = document.createElement("div"); badge.className = "queued-badge";
	const label = document.createElement("span"); label.className = "queued-label"; label.textContent = "Queued";
	const btn = document.createElement("button"); btn.className = "queued-cancel"; btn.title = "Cancel all queued"; btn.textContent = "\u2715";
	btn.addEventListener("click", (e: MouseEvent) => { e.stopPropagation(); sendRpc("chat.cancel_queued", { sessionKey }); });
	badge.appendChild(label); badge.appendChild(btn); el.appendChild(badge);
	tray.appendChild(el); tray.classList.remove("hidden");
}

function chatAutoResize(): void {
	if (!S.chatInput) return;
	S.chatInput.style.height = "auto";
	S.chatInput.style.height = `${Math.min(S.chatInput.scrollHeight, 120)}px`;
}

function handleHistoryUp(): void {
	if (S.chatHistory.length === 0) return;
	if (S.chatHistoryIdx === -1) { S.setChatHistoryDraft(S.chatInput.value); S.setChatHistoryIdx(S.chatHistory.length - 1); }
	else if (S.chatHistoryIdx > 0) S.setChatHistoryIdx(S.chatHistoryIdx - 1);
	S.chatInput.value = S.chatHistory[S.chatHistoryIdx];
	chatAutoResize();
}

function handleHistoryDown(): void {
	if (S.chatHistoryIdx === -1) return;
	if (S.chatHistoryIdx < S.chatHistory.length - 1) {
		S.setChatHistoryIdx(S.chatHistoryIdx + 1);
		S.chatInput.value = S.chatHistory[S.chatHistoryIdx];
	} else { S.setChatHistoryIdx(-1); S.chatInput.value = S.chatHistoryDraft; }
	chatAutoResize();
}

// Safe: static hardcoded HTML template string — no user input is interpolated.
// This is a compile-time constant defined in the original JS source.
const chatPageHTML =
	'<div style="position:absolute;inset:0;display:grid;grid-template-rows:auto auto 1fr auto auto auto;overflow:hidden">' +
	'<div class="chat-toolbar h-12 px-4 border-b border-[var(--border)] bg-[var(--surface)] flex items-center gap-2" style="grid-row:1;">' +
	'<div id="modelCombo" class="model-combo"><button id="modelComboBtn" class="model-combo-btn" type="button"><span id="modelComboLabel">loading\u2026</span><span class="icon icon-sm icon-chevron-down model-combo-chevron"></span></button><div id="modelDropdown" class="model-dropdown hidden"><input id="modelSearchInput" type="text" placeholder="Search models\u2026" class="model-search-input" autocomplete="off" /><div id="modelDropdownList" class="model-dropdown-list"></div></div></div>' +
	'<div id="reasoningCombo" class="model-combo hidden"><button id="reasoningComboBtn" class="model-combo-btn" type="button" title="Reasoning effort"><span class="icon icon-sm icon-brain" style="flex-shrink:0;"></span><span id="reasoningComboLabel">Off</span><span class="icon icon-sm icon-chevron-down model-combo-chevron"></span></button><div id="reasoningDropdown" class="model-dropdown hidden"><div id="reasoningDropdownList" class="model-dropdown-list"></div></div></div>' +
	'<div id="nodeCombo" class="model-combo hidden"><button id="nodeComboBtn" class="model-combo-btn" type="button"><span class="icon icon-sm icon-server" style="flex-shrink:0;"></span><span id="nodeComboLabel">Local</span><span class="icon icon-sm icon-chevron-down model-combo-chevron"></span></button><div id="nodeDropdown" class="model-dropdown hidden" tabindex="-1"><div id="nodeDropdownList" class="model-dropdown-list"></div></div></div>' +
	'<div id="sessionHeaderToolbarMount" class="ml-auto flex items-center gap-1.5"></div>' +
	'<button id="chatMoreBtn" type="button" class="model-combo-btn" title="More controls" aria-label="More controls"><span class="icon icon-lg icon-menu-dots-horizontal"></span></button></div>' +
	'<div id="chatMoreModal" class="provider-modal-backdrop hidden"><div class="provider-modal" style="width:560px;max-width:92vw;"><div class="provider-modal-header"><div class="flex items-center gap-2"><button id="chatMoreDeleteAllBtn" type="button" class="provider-btn provider-btn-sm chat-session-btn-danger inline-flex items-center gap-1.5" style="background:var(--error);border-color:var(--error);color:#fff;"><span class="icon icon-sm icon-x-circle shrink-0"></span><span id="chatMoreDeleteAllLabel">Delete all sessions</span></button></div><div id="sessionHeaderModalTopMount" class="flex items-center gap-2"></div></div><div class="provider-modal-body flex flex-col gap-3"><div class="flex flex-wrap items-center gap-2"><button id="sandboxToggle" class="sandbox-toggle text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1" title="Toggle sandbox mode"><span class="icon icon-md icon-lock shrink-0"></span><span id="sandboxLabel">sandboxed</span></button><div style="position:relative;display:inline-block"><button id="sandboxImageBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Sandbox image"><span class="icon icon-md icon-cube shrink-0"></span><span id="sandboxImageLabel" class="max-w-[120px] truncate">ubuntu:25.10</span></button><div id="sandboxImageDropdown" class="hidden" style="position:absolute;top:100%;left:0;z-index:50;margin-top:4px;min-width:200px;max-height:300px;overflow-y:auto;background:var(--surface);border:1px solid var(--border);border-radius:8px;box-shadow:0 4px 12px rgba(0,0,0,.15);"></div></div><button id="mcpToggleBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1" title="Toggle MCP tools for this session"><span class="icon icon-md icon-link shrink-0"></span><span id="mcpToggleLabel">MCP</span></button><button id="debugPanelBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Show context debug info"><span class="icon icon-md icon-wrench shrink-0"></span><span id="debugPanelLabel">Debug</span></button><button id="fullContextBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Show full LLM context (system prompt + history)"><span class="icon icon-md icon-document shrink-0"></span><span id="fullContextLabel">Context</span></button></div><div id="sessionControlsSection" class="border-t border-[var(--border)] pt-3"><div id="sessionHeaderModalMount" class="w-full"></div></div></div></div></div>' +
	'<div id="debugModal" class="provider-modal-backdrop hidden"><div class="provider-modal" style="width:min(980px,96vw);max-width:96vw;max-height:88vh;"><div class="provider-modal-header"><div class="provider-item-name">Debug context</div><button id="debugModalCloseBtn" type="button" class="provider-btn provider-btn-secondary provider-btn-sm">Close</button></div><div class="provider-modal-body" style="padding:0;overflow:hidden;"><div id="debugPanel" class="px-4 py-3 overflow-y-auto" style="max-height:72vh;"></div></div></div></div>' +
	'<div id="fullContextModal" class="provider-modal-backdrop hidden"><div class="provider-modal" style="width:min(1080px,96vw);max-width:96vw;max-height:88vh;"><div class="provider-modal-header"><div class="provider-item-name">Full context</div><button id="fullContextModalCloseBtn" type="button" class="provider-btn provider-btn-secondary provider-btn-sm">Close</button></div><div class="provider-modal-body" style="padding:0;overflow:hidden;"><div id="fullContextPanel" class="px-4 py-3 overflow-y-auto" style="max-height:72vh;"></div></div></div></div>' +
	'<div class="p-4 flex flex-col gap-2" id="messages" style="grid-row:3;overflow-y:auto;min-height:0"></div>' +
	'<div id="queuedMessages" class="queued-tray hidden" style="grid-row:4;"></div>' +
	'<div id="tokenBar" class="token-bar" style="grid-row:5;"></div>' +
	'<div class="chat-input-row px-4 py-3 border-t border-[var(--border)] bg-[var(--surface)] flex gap-2 items-end" style="grid-row:6;"><span id="chatCommandPrompt" class="chat-command-prompt chat-command-prompt-hidden" title="Command prompt symbol" aria-hidden="true">$</span><textarea id="chatInput" placeholder="Type a message..." rows="1" enterkeyhint="send" class="flex-1 bg-[var(--surface2)] border border-[var(--border)] text-[var(--text)] px-3 py-2 rounded-lg text-sm resize-none min-h-[40px] max-h-[120px] leading-relaxed focus:outline-none focus:border-[var(--border-strong)] focus:ring-1 focus:ring-[var(--accent-subtle)] transition-colors font-[var(--font-body)]"></textarea><button id="micBtn" disabled title="Click to start recording" class="mic-btn min-h-[40px] px-3 bg-[var(--surface2)] border border-[var(--border)] rounded-lg text-[var(--muted)] cursor-pointer disabled:opacity-40 disabled:cursor-default transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text)]"><span class="icon icon-lg icon-microphone"></span></button><button id="sendBtn" disabled class="provider-btn min-h-[40px] disabled:opacity-40 disabled:cursor-default">Send</button></div></div>';

function msgRole(el: Element): string | null {
	if (el.classList.contains("user")) return "You";
	if (el.classList.contains("assistant")) return "Assistant";
	return null;
}

function handleChatCopy(e: ClipboardEvent): void {
	const sel = window.getSelection();
	if (!sel || sel.isCollapsed || !S.chatMsgBox) return;
	const lines: string[] = [];
	for (const msg of S.chatMsgBox.querySelectorAll(".msg")) {
		if (!sel.containsNode(msg, true)) continue;
		const role = msgRole(msg);
		if (!role) continue;
		const text = sel.containsNode(msg, false) ? (msg.textContent || "").trim() : sel.toString().trim();
		if (text) lines.push(`${role}:\n${text}`);
	}
	if (lines.length > 1) { e.preventDefault(); e.clipboardData?.setData("text/plain", lines.join("\n\n")); }
}

function mountSessionHeaderControls(closeChatMore: () => void): void {
	const headerToolbarMount = S.$("sessionHeaderToolbarMount");
	if (headerToolbarMount) {
		render(
			<SessionHeader showName={false} showShare={false} showFork={false} showClear={false} showDelete={false} showArchive={false} />,
			headerToolbarMount,
		);
	}
	const headerModalMount = S.$("sessionHeaderModalMount");
	if (headerModalMount) {
		render(
			<SessionHeader showSelectors={false} showStop={false} showFork={false} showShare={false} showDelete={false} showArchive={false} nameOwnLine={true} showRenameButton={true} />,
			headerModalMount,
		);
	}
	const headerModalTopMount = S.$("sessionHeaderModalTopMount");
	if (headerModalTopMount) {
		render(
			<SessionHeader showSelectors={false} showName={false} showStop={false} actionButtonClass={"provider-btn provider-btn-secondary provider-btn-sm"} onBeforeShare={() => closeChatMore?.()} onBeforeArchive={() => closeChatMore?.()} onBeforeDelete={() => closeChatMore?.()} />,
			headerModalTopMount,
		);
	}
}

function bindSessionControlsVisibility(): void {
	const sec = S.$("sessionControlsSection") as HTMLElement | null;
	if (!sec) return;
	disposeSessionControlsVisibility?.();
	disposeSessionControlsVisibility = effect(() => {
		const isMain = (sessionStore.activeSessionKey.value || "main") === "main";
		sec.classList.toggle("hidden", isMain);
	});
}

function bindChatMoreModal(debugModal: HTMLElement | null, fullContextModal: HTMLElement | null, closeDebugModal: (() => void) | null, closeFullContextModal: (() => void) | null): (() => void) | null {
	const chatMoreModal = S.$("chatMoreModal") as HTMLElement | null;
	const chatMoreBtn = S.$("chatMoreBtn") as HTMLElement | null;
	if (!(chatMoreModal && chatMoreBtn)) return null;
	const closeChatMore = (): void => {
		chatMoreModal.classList.add("hidden"); chatMoreBtn.classList.remove("active");
		if (S.sandboxImageDropdown) S.sandboxImageDropdown.classList.add("hidden");
	};
	const openChatMore = (): void => { setDebugModalOpen(false); setFullContextModalOpen(false); chatMoreModal.classList.remove("hidden"); chatMoreBtn.classList.add("active"); };
	chatMoreBtn.addEventListener("click", openChatMore);
	chatMoreModal.addEventListener("click", (e: MouseEvent) => { if (e.target === chatMoreModal) closeChatMore(); });
	for (const id of ["debugPanelBtn", "fullContextBtn"]) {
		const b = S.$(id); if (b) b.addEventListener("click", closeChatMore);
	}
	chatMoreModalKeydownHandler = (e: KeyboardEvent): void => {
		if (e.key !== "Escape") return;
		if (fullContextModal && !fullContextModal.classList.contains("hidden")) { closeFullContextModal?.(); return; }
		if (debugModal && !debugModal.classList.contains("hidden")) { closeDebugModal?.(); return; }
		closeChatMore();
	};
	document.addEventListener("keydown", chatMoreModalKeydownHandler);
	return closeChatMore;
}

function bindDeleteAllSessions(closeChatMore: (() => void) | null): void {
	const btn = S.$("chatMoreDeleteAllBtn") as HTMLButtonElement | null;
	if (!btn) return;
	const label = S.$("chatMoreDeleteAllLabel") as HTMLElement | null;
	let inFlight = false;
	btn.addEventListener("click", () => {
		if (inFlight) return;
		inFlight = true; btn.disabled = true;
		if (label) label.textContent = "Deleting\u2026";
		closeChatMore?.();
		clearAllSessions()
			.then((res: any) => { if (res?.ok && !res?.skipped) return; if (res?.cancelled || res?.skipped) return; chatAddMsg("error", res?.error?.message || "Failed to clear sessions"); })
			.finally(() => { inFlight = false; btn.disabled = false; if (label) label.textContent = "Delete all sessions"; });
	});
}

function bindChatComposer(): void {
	S.chatInput.addEventListener("input", () => { chatAutoResize(); slashHandleInput(); });
	S.chatInput.addEventListener("keydown", (e: KeyboardEvent) => {
		if (slashHandleKeydown(e)) return;
		if (e.key === "Escape" && S.commandModeEnabled && !S.chatInput.value.trim()) { e.preventDefault(); setCommandMode(false); return; }
		if (e.key === "Enter" && !e.shiftKey && !(e as any).isComposing) { e.preventDefault(); sendChat(); return; }
		if (e.key === "ArrowUp" && S.chatInput.selectionStart === 0 && !e.shiftKey) { e.preventDefault(); handleHistoryUp(); return; }
		if (e.key === "ArrowDown" && S.chatInput.selectionStart === S.chatInput.value.length && !e.shiftKey) { e.preventDefault(); handleHistoryDown(); }
	});
	S.chatSendBtn.addEventListener("click", sendChat);
}

function initializeChatControls(): void {
	S.setModelCombo(S.$("modelCombo")); S.setModelComboBtn(S.$("modelComboBtn")); S.setModelComboLabel(S.$("modelComboLabel"));
	S.setModelDropdown(S.$("modelDropdown")); S.setModelSearchInput(S.$("modelSearchInput")); S.setModelDropdownList(S.$("modelDropdownList"));
	bindModelComboEvents();
	bindReasoningToggle();
	S.setNodeCombo(S.$("nodeCombo")); S.setNodeComboBtn(S.$("nodeComboBtn")); S.setNodeComboLabel(S.$("nodeComboLabel"));
	S.setNodeDropdown(S.$("nodeDropdown")); S.setNodeDropdownList(S.$("nodeDropdownList"));
	bindNodeComboEvents(); fetchNodes();
	S.setSandboxToggleBtn(S.$("sandboxToggle")); S.setSandboxLabel(S.$("sandboxLabel"));
	bindSandboxToggleEvents(); updateSandboxUI(true);
	S.setSandboxImageBtn(S.$("sandboxImageBtn")); S.setSandboxImageLabel(S.$("sandboxImageLabel")); S.setSandboxImageDropdown(S.$("sandboxImageDropdown"));
	bindSandboxImageEvents(); updateSandboxImageUI(null);
}

function bindContextModals(): { debugModal: HTMLElement | null; fullContextModal: HTMLElement | null; closeDebugModal: (() => void) | null; closeFullContextModal: (() => void) | null } {
	const debugModal = S.$("debugModal") as HTMLElement | null;
	const debugCloseBtn = S.$("debugModalCloseBtn") as HTMLElement | null;
	let closeDebugModal: (() => void) | null = null;
	if (debugModal) {
		closeDebugModal = () => setDebugModalOpen(false);
		if (debugCloseBtn) debugCloseBtn.addEventListener("click", closeDebugModal);
		debugModal.addEventListener("click", (e: MouseEvent) => { if (e.target === debugModal) closeDebugModal!(); });
	}
	const fullContextModal = S.$("fullContextModal") as HTMLElement | null;
	const fcCloseBtn = S.$("fullContextModalCloseBtn") as HTMLElement | null;
	let closeFullContextModal: (() => void) | null = null;
	if (fullContextModal) {
		closeFullContextModal = () => setFullContextModalOpen(false);
		if (fcCloseBtn) fcCloseBtn.addEventListener("click", closeFullContextModal);
		fullContextModal.addEventListener("click", (e: MouseEvent) => { if (e.target === fullContextModal) closeFullContextModal!(); });
	}
	return { debugModal, fullContextModal, closeDebugModal, closeFullContextModal };
}

function syncModelComboLabel(): void {
	if (!(S.models.length > 0 && S.modelComboLabel)) return;
	const found = S.models.find((m: any) => m.id === S.selectedModelId);
	if (found) { S.modelComboLabel.textContent = found.displayName || found.id; return; }
	if (S.models[0]) S.modelComboLabel.textContent = S.models[0].displayName || S.models[0].id;
}

function resolveInitialSessionKey(sessionKeyFromUrl: string | null): string {
	if (sessionKeyFromUrl) return sessionKeyFromUrl;
	const sk = localStorage.getItem("moltis-session") || "main";
	history.replaceState(null, "", sessionPath(sk));
	return sk;
}

function startInitialChatSession(sessionKey: string): void {
	if (!S.connected) return;
	S.chatSendBtn.disabled = false;
	switchSession(sessionKey);
}

function initializeChatMediaDrop(): void {
	if (window.innerWidth < 768) return;
	const inputArea = S.chatInput?.closest(".px-4.py-3");
	initMediaDrop(S.chatMsgBox, inputArea as HTMLElement | null);
}

registerPrefix(
	routes.chats,
	function initChat(container: HTMLElement, sessionKeyFromUrl: string | null) {
		container.style.cssText = "position:relative";
		// Safe: chatPageHTML is a static hardcoded template with no user input.
		// This is a compile-time constant defined above -- no dynamic or user data.
		container.innerHTML = chatPageHTML;

		S.setChatMsgBox(S.$("messages")); S.setChatInput(S.$("chatInput")); S.setChatSendBtn(S.$("sendBtn"));
		updateCommandInputUI();
		initializeChatControls();

		let closeChatMore: (() => void) | null = null;
		mountSessionHeaderControls(() => closeChatMore?.());
		bindSessionControlsVisibility();

		const mcpToggle = S.$("mcpToggleBtn");
		if (mcpToggle) mcpToggle.addEventListener("click", toggleMcp);
		updateMcpToggleUI(true);

		const mb = bindContextModals();
		closeChatMore = bindChatMoreModal(mb.debugModal, mb.fullContextModal, mb.closeDebugModal, mb.closeFullContextModal);
		bindDeleteAllSessions(closeChatMore);

		const debugBtn = S.$("debugPanelBtn");
		if (debugBtn) debugBtn.addEventListener("click", toggleDebugPanel);
		S.$("fullContextBtn")?.addEventListener("click", toggleFullContextPanel);

		syncModelComboLabel();
		const sessionKey = resolveInitialSessionKey(sessionKeyFromUrl);
		startInitialChatSession(sessionKey);
		bindChatComposer();
		S.chatMsgBox.addEventListener("copy", handleChatCopy);
		initVoiceInput(S.$("micBtn") as HTMLButtonElement | null);
		initializeChatMediaDrop();
		S.chatInput.focus();
	},
	function teardownChat() {
		teardownVoiceInput(); teardownMediaDrop(); unbindReasoningToggle(); unbindNodeEvents(); slashHideMenu();
		if (chatMoreModalKeydownHandler) { document.removeEventListener("keydown", chatMoreModalKeydownHandler); chatMoreModalKeydownHandler = null; }
		disposeSessionControlsVisibility?.(); disposeSessionControlsVisibility = null;
		const m1 = S.$("sessionHeaderToolbarMount"); if (m1) render(null, m1);
		const m2 = S.$("sessionHeaderModalMount"); if (m2) render(null, m2);
		const m3 = S.$("sessionHeaderModalTopMount"); if (m3) render(null, m3);
		S.setChatMsgBox(null); S.setChatInput(null); S.setChatSendBtn(null); S.setStreamEl(null); S.setStreamText("");
		S.setModelCombo(null); S.setModelComboBtn(null); S.setModelComboLabel(null); S.setModelDropdown(null); S.setModelSearchInput(null); S.setModelDropdownList(null);
		S.setNodeCombo(null); S.setNodeComboBtn(null); S.setNodeComboLabel(null); S.setNodeDropdown(null); S.setNodeDropdownList(null);
		S.setSandboxToggleBtn(null); S.setSandboxLabel(null);
	},
);
