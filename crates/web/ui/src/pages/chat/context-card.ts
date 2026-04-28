// ── Context card rendering ───────────────────────────────────

import { smartScrollToBottom } from "../../chat-ui";
import { formatBytes, formatTokens } from "../../helpers";
import * as S from "../../state";
import { slashInjectStyles } from "./slash-commands";

// ── Types ────────────────────────────────────────────────────

interface ContextFile {
	path: string;
	size?: number;
}

export interface SessionData {
	key?: string;
	messageCount?: number;
	model?: string;
	provider?: string;
	label?: string;
}

export interface ProjectData {
	label?: string;
	directory?: string;
	systemPrompt?: string;
	contextFiles?: ContextFile[];
}

export interface SandboxData {
	enabled?: boolean;
	backend?: string;
	mode?: string;
	scope?: string;
	workspaceMount?: string;
	image?: string;
	containerName?: string;
}

export interface ExecutionData {
	mode?: string;
	promptSymbol?: string;
}

export interface TokenUsageData {
	inputTokens?: number;
	outputTokens?: number;
	cacheReadTokens?: number;
	cacheWriteTokens?: number;
	total?: number;
	currentInputTokens?: number;
	currentOutputTokens?: number;
	currentCacheReadTokens?: number;
	currentCacheWriteTokens?: number;
	currentTotal?: number;
	estimatedNextInputTokens?: number;
	contextWindow?: number;
}

export interface WorkspaceFile {
	name?: string;
	truncated?: boolean;
	original_chars?: number;
	limit_chars?: number;
	truncated_chars?: number;
}

export interface PromptMemoryData {
	mode?: string;
	present?: boolean;
	chars?: number;
	fileSource?: string;
	path?: string;
	snapshotActive?: boolean;
}

export interface ContextData {
	session?: SessionData;
	project?: ProjectData | null;
	tools?: Array<{ name: string; description?: string }>;
	skills?: Array<{ name: string; description?: string; source?: string }>;
	mcpServers?: Array<{ name: string; state?: string; tool_count?: number }>;
	mcpDisabled?: boolean;
	sandbox?: SandboxData;
	execution?: ExecutionData;
	tokenUsage?: TokenUsageData;
	promptMemory?: PromptMemoryData | null;
	supportsTools?: boolean;
}

// ── DOM helpers ──────────────────────────────────────────────

export function ctxEl(tag: string, cls: string, text?: string): HTMLElement {
	const el = document.createElement(tag);
	if (cls) el.className = cls;
	if (text !== undefined) el.textContent = text;
	return el;
}

export function ctxRow(label: string, value: string, mono?: boolean): HTMLElement {
	const row = ctxEl("div", "ctx-row");
	row.appendChild(ctxEl("span", "ctx-label", label));
	row.appendChild(ctxEl("span", `ctx-value${mono ? " mono" : ""}`, value));
	return row;
}

export function ctxSection(title: string): HTMLElement {
	const sec = ctxEl("div", "ctx-section");
	sec.appendChild(ctxEl("div", "ctx-section-title", title));
	return sec;
}

// ── Prompt memory helpers ────────────────────────────────────

export function formatPromptMemoryMode(mode: string | undefined): string {
	if (mode === "frozen-at-session-start") return "Frozen at session start";
	if (mode === "live-reload") return "Live reload";
	return mode || "unknown";
}

export function formatPromptMemorySource(source: string | undefined): string {
	if (source === "agent_workspace") return "Agent workspace";
	if (source === "root_workspace") return "Root workspace";
	return source || "unknown";
}

export function buildPromptMemorySummary(promptMemory: PromptMemoryData | null): string {
	if (!promptMemory) return "Unavailable";
	const parts: string[] = [formatPromptMemoryMode(promptMemory.mode)];
	if (promptMemory.snapshotActive) parts.push("snapshot active");
	parts.push(promptMemory.present ? `${Number(promptMemory.chars || 0).toLocaleString()} chars` : "empty");
	return parts.join(" \u00b7 ");
}

export function promptMemoryDetailParts(promptMemory: PromptMemoryData | null): string[] {
	if (!promptMemory) return [];
	const parts: string[] = [];
	if (promptMemory.fileSource) parts.push(`source ${formatPromptMemorySource(promptMemory.fileSource)}`);
	if (promptMemory.path) parts.push(promptMemory.path);
	return parts;
}

// ── Section renderers ────────────────────────────────────────

export function renderContextSessionSection(card: HTMLElement, data: ContextData): void {
	const sess = data.session ?? {};
	const sec = ctxSection("Session");
	sec.appendChild(ctxRow("Key", sess.key || "unknown", true));
	sec.appendChild(ctxRow("Messages", String(sess.messageCount || 0)));
	sec.appendChild(ctxRow("Model", sess.model || "default", true));
	if (sess.provider) sec.appendChild(ctxRow("Provider", sess.provider, true));
	if (sess.label) sec.appendChild(ctxRow("Label", sess.label));
	sec.appendChild(ctxRow("Tool Support", data.supportsTools === false ? "Disabled" : "Enabled"));
	card.appendChild(sec);
}

export function renderContextProjectSection(card: HTMLElement, data: ContextData): void {
	const proj = data.project;
	const sec = ctxSection("Project");
	if (proj) {
		sec.appendChild(ctxRow("Name", proj.label || "(unnamed)"));
		if (proj.directory) sec.appendChild(ctxRow("Directory", proj.directory, true));
		if (proj.systemPrompt) sec.appendChild(ctxRow("System Prompt", `${proj.systemPrompt.length} chars`));
		const ctxFiles: ContextFile[] = proj.contextFiles || [];
		if (ctxFiles.length > 0) {
			const fl = ctxEl("div", "ctx-section-title", `Context Files (${ctxFiles.length})`);
			fl.classList.add("spaced");
			sec.appendChild(fl);
			ctxFiles.forEach((f) => {
				const row = ctxEl("div", "ctx-file");
				row.appendChild(ctxEl("span", "ctx-file-path", f.path));
				row.appendChild(ctxEl("span", "ctx-file-size", formatBytes(f.size ?? 0)));
				sec.appendChild(row);
			});
		}
	} else {
		sec.appendChild(ctxEl("div", "ctx-empty", "No project bound to this session"));
	}
	card.appendChild(sec);
}

export function renderContextToolsSection(card: HTMLElement, data: ContextData): void {
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

export function renderContextSkillsSection(card: HTMLElement, data: ContextData): void {
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
			dot.style.background = isPlugin ? "var(--accent)" : "var(--success, #4a9)";
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

export function renderContextMcpSection(card: HTMLElement, data: ContextData): void {
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
				dot.style.background = "var(--ok)";
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

export function renderContextSandboxSection(card: HTMLElement, data: ContextData): void {
	const sb = data.sandbox ?? {};
	const exec = data.execution ?? {};
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

export function renderContextTokensSection(card: HTMLElement, data: ContextData): void {
	const tu = data.tokenUsage ?? {};
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
	const contextWindow = tu.contextWindow ?? 0;
	if (contextWindow > 0) {
		const pct = Math.max(0, 100 - Math.round((estimatedNextInput / contextWindow) * 100));
		sec.appendChild(ctxRow("Context left", `${pct}% of ${formatTokens(contextWindow)}`, true));
	}
	card.appendChild(sec);
}

export function renderContextPromptMemorySection(card: HTMLElement, data: ContextData): void {
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

// ── Main context card renderer ───────────────────────────────

export function renderContextCard(data: ContextData): void {
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
	smartScrollToBottom();
}
