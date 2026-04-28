// ── Chat page (Preact + JSX) ────────────────────────────────────────
// This is a TypeScript/JSX conversion of page-chat.js. The page is
// heavily imperative (DOM manipulation + registerPrefix router pattern)
// so the conversion preserves that style while adding types.

// NOTE: The chatPageHTML constant uses innerHTML assignment which is safe
// because it is a compile-time static string with no user input interpolated.
// The original JS file documents this explicitly. The eslint-disable comment
// is preserved from the original source.

import { render } from "preact";
import { chatAddMsg, hideNewContentIndicator, isChatAtBottom, smartScrollToBottom } from "../chat-ui";
import { SessionHeader } from "../components/SessionHeader";
import { formatTokens, sendRpc } from "../helpers";
import { initMediaDrop, teardownMediaDrop } from "../media-drop";
import { bindModelComboEvents } from "../models";
import { bindNodeComboEvents, fetchNodes, unbindNodeEvents } from "../nodes-selector";
import { bindProjectComboEvents } from "../project-combo";
import { fetchProjects } from "../projects";
import { bindReasoningToggle, unbindReasoningToggle } from "../reasoning-toggle";
import { registerPrefix, sessionPath } from "../router";
import { routes } from "../routes";
import { bindSandboxImageEvents, bindSandboxToggleEvents, updateSandboxImageUI, updateSandboxUI } from "../sandbox";
import { switchSession } from "../sessions";
import * as S from "../state";
import { initVadButton, initVoiceInput, teardownVoiceInput } from "../voice-input";
import {
	chatAutoResize,
	handleHistoryDown,
	handleHistoryUp,
	sendChat,
	setMaybeRefreshFullContextFn,
} from "./chat/chat-send";
import {
	buildPromptMemorySummary,
	ctxEl,
	ctxRow,
	ctxSection,
	type PromptMemoryData,
	promptMemoryDetailParts,
	renderContextMcpSection,
	renderContextProjectSection,
	renderContextPromptMemorySection,
	renderContextSandboxSection,
	renderContextSessionSection,
	renderContextSkillsSection,
	renderContextTokensSection,
	renderContextToolsSection,
} from "./chat/context-card";
// ── Sub-module imports ──────────────────────────────────────
import {
	setSendChatFn,
	slashHandleInput,
	slashHandleKeydown,
	slashHideMenu,
	slashInjectStyles,
} from "./chat/slash-commands";

// ── Module state ─────────────────────────────────────────────
let promptMemoryToolbarRequestId = 0;
let contextModalsKeydownHandler: ((e: KeyboardEvent) => void) | null = null;

// ── Prompt memory toolbar helpers ─────────────────────────────

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

// ── Compact card ─────────────────────────────────────────────

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
	smartScrollToBottom();
}

// ── Debug / full context panels ──────────────────────────────

interface ContextMessage {
	role?: string;
	content?: unknown;
	tool_calls?: Array<{
		id?: string;
		function?: { name?: string; arguments?: string };
	}>;
	tool_call_id?: string;
}

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
	if (!opening) {
		setDebugModalOpen(false);
		return;
	}
	setFullContextModalOpen(false);
	setDebugModalOpen(true);
	refreshDebugPanel();
}

// ── Full context panel ───────────────────────────────────────
const ROLE_COLORS: Record<string, string> = {
	system: "var(--accent)",
	user: "var(--ok, #22c55e)",
	assistant: "var(--info, #3b82f6)",
	tool: "var(--muted)",
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
		try {
			pre.textContent = JSON.stringify(JSON.parse(tc.function.arguments), null, 2);
		} catch {
			pre.textContent = tc.function.arguments;
		}
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
		const pre = ctxEl(
			"pre",
			"text-xs font-mono whitespace-pre-wrap break-words bg-[var(--surface)] border border-[var(--border)] rounded-md p-2 text-[var(--text)]",
		);
		pre.textContent = contentStr;
		body.appendChild(pre);
	}
	for (const tc of msg.tool_calls || []) body.appendChild(ctxMsgToolCall(tc));
	wrapper.appendChild(body);
	return wrapper;
}

function buildFullContextPromptMemoryBox(pm: PromptMemoryData | null): HTMLElement | null {
	if (!pm) return null;
	const box = ctxEl(
		"div",
		"text-xs mb-3 rounded-md border border-[var(--border)] bg-[var(--surface)] p-2 text-[var(--text)]",
	);
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
	const warning = ctxEl(
		"div",
		"text-xs mb-3 rounded-md border border-[var(--border)] bg-[var(--surface)] p-2 text-[var(--text)]",
	);
	warning.textContent = tf
		.map((f: any) => {
			const name = typeof f.name === "string" ? f.name : "workspace file";
			return `${name}: ${Number(f.original_chars || 0).toLocaleString()} chars, limit ${Number(f.limit_chars || 0).toLocaleString()}, truncated by ${Number(f.truncated_chars || 0).toLocaleString()}`;
		})
		.join(" | ");
	panel.appendChild(warning);
}

function buildFullContextHeaderRow(
	payload: any,
	onRefresh: (btn: HTMLButtonElement) => void,
): { headerRow: HTMLElement; copyBtn: HTMLElement; downloadBtn: HTMLElement; llmOutputBtn: HTMLElement } {
	const headerRow = ctxEl("div", "flex items-center gap-3 mb-3");
	const headerText = ctxEl("span", "text-xs text-[var(--muted)]");
	headerText.textContent = `${payload.messageCount} messages \u00b7 system prompt ${payload.systemPromptChars.toLocaleString()} chars \u00b7 total ${payload.totalChars.toLocaleString()} chars`;
	headerRow.appendChild(headerText);
	const copyBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm");
	copyBtn.textContent = "Copy";
	const downloadBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm");
	downloadBtn.textContent = "Download";
	const llmOutputBtn = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm");
	llmOutputBtn.textContent = "LLM output";
	headerRow.appendChild(copyBtn);
	headerRow.appendChild(downloadBtn);
	headerRow.appendChild(llmOutputBtn);
	const pm = payload.promptMemory || null;
	if (pm?.mode === "frozen-at-session-start") {
		const rb = ctxEl("button", "provider-btn provider-btn-secondary provider-btn-sm") as HTMLButtonElement;
		rb.textContent = "Refresh memory";
		rb.addEventListener("click", () => onRefresh(rb));
		headerRow.appendChild(rb);
	}
	return { headerRow, copyBtn, downloadBtn, llmOutputBtn };
}

function wireFullContextCopyButton(
	copyBtn: HTMLElement,
	messages: ContextMessage[],
	llmOutputs: any[],
	llmOutputPanel: HTMLElement,
): void {
	copyBtn.addEventListener("click", () => {
		const lines = messages.map((m) => {
			const content = typeof m.content === "string" ? m.content : JSON.stringify(m.content);
			const parts = [content];
			for (const tc of m.tool_calls || [])
				parts.push(`[tool_call: ${tc.function?.name || "?"} ${tc.function?.arguments || ""}]`);
			return `[${m.role}] ${parts.join("\n")}`;
		});
		const contextText = lines.join("\n");
		let copyText = contextText;
		const llmOutputVisible = llmOutputPanel && !llmOutputPanel.classList.contains("hidden");
		if (llmOutputVisible) copyText = `LLM output:\n${JSON.stringify(llmOutputs, null, 2)}\n\nContext:\n${contextText}`;
		navigator.clipboard.writeText(copyText).then(() => {
			copyBtn.textContent = "Copied!";
			setTimeout(() => {
				copyBtn.textContent = "Copy";
			}, 1500);
		});
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
	panel.appendChild(
		ctxEl(
			"div",
			"text-xs text-[var(--muted)] mb-1",
			`${llmOutputs.length} assistant output${llmOutputs.length === 1 ? "" : "s"}`,
		),
	);
	const pre = ctxEl(
		"pre",
		"text-xs font-mono whitespace-pre-wrap break-words bg-[var(--surface)] border border-[var(--border)] rounded-md p-2 text-[var(--text)]",
	);
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
	refreshBtn.disabled = true;
	refreshBtn.textContent = "Refreshing\u2026";
	refreshPromptMemoryToolbarSnapshot().then(() => {
		refreshBtn.disabled = false;
		refreshBtn.textContent = "Refresh memory";
	});
}

function refreshFullContextPanel(): void {
	const panel = S.$("fullContextPanel") as HTMLElement | null;
	if (!panel) return;
	panel.textContent = "";
	panel.appendChild(ctxEl("div", "text-xs text-[var(--muted)]", "Building full context\u2026"));
	sendRpc("chat.full_context", {}).then((res: any) => {
		panel.textContent = "";
		if (!(res?.ok && res.payload)) {
			panel.appendChild(ctxEl("div", "text-xs text-[var(--error)]", "Failed to build context"));
			return;
		}
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
	if (!opening) {
		setFullContextModalOpen(false);
		return;
	}
	setDebugModalOpen(false);
	setFullContextModalOpen(true);
	refreshFullContextPanel();
}

export function maybeRefreshFullContext(): void {
	const modal = S.$("fullContextModal") as HTMLElement | null;
	if (modal && !modal.classList.contains("hidden")) refreshFullContextPanel();
}

// ── MCP toggle ───────────────────────────────────────────────
export function updateMcpToggleUI(enabled: boolean): void {
	const btn = S.$("mcpToggleBtn") as HTMLElement | null;
	const label = S.$("mcpToggleLabel") as HTMLElement | null;
	if (!btn) return;
	if (enabled) {
		btn.style.color = "var(--ok)";
		btn.style.borderColor = "var(--ok)";
		if (label) label.textContent = "MCP";
		btn.title = "MCP tools enabled \u2014 click to disable for this session";
	} else {
		btn.style.color = "var(--muted)";
		btn.style.borderColor = "var(--border)";
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

interface ModelNotice {
	id: string;
	displayName?: string;
	provider?: string;
	supportsTools?: boolean;
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
	smartScrollToBottom();
}

// ── Chat copy handler ───────────────────────────────────────

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
	if (lines.length > 1) {
		e.preventDefault();
		e.clipboardData?.setData("text/plain", lines.join("\n\n"));
	}
}

// ── Session header controls ──────────────────────────────────

function mountSessionHeaderControls(): void {
	const sessionNameMount = S.$("sessionNameMount");
	if (sessionNameMount) {
		render(
			<SessionHeader
				showSelectors={false}
				showName={true}
				showShare={false}
				showFork={false}
				showStop={false}
				showClear={false}
				showDelete={false}
				showArchive={false}
			/>,
			sessionNameMount,
		);
	}
	const headerToolbarMount = S.$("sessionHeaderToolbarMount");
	if (headerToolbarMount) {
		render(
			<SessionHeader
				showName={false}
				showFork={false}
				showShare={false}
				showClear={false}
				showDelete={false}
				showArchive={false}
				showStop={false}
			/>,
			headerToolbarMount,
		);
	}
	const sessionActionsMount = S.$("sessionActionsMount");
	if (sessionActionsMount) {
		render(
			<SessionHeader
				showSelectors={false}
				showName={false}
				showStop={false}
				actionButtonClass={
					"text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] text-[var(--muted)]"
				}
			/>,
			sessionActionsMount,
		);
	}
}

function bindChatComposer(): void {
	const chatInput = S.chatInput as HTMLTextAreaElement;
	chatInput.addEventListener("input", () => {
		chatAutoResize();
		slashHandleInput();
	});
	chatInput.addEventListener("keydown", (e: KeyboardEvent) => {
		if (slashHandleKeydown(e)) return;
		if (e.key === "Escape" && S.commandModeEnabled && !chatInput.value.trim()) {
			e.preventDefault();
			S.setCommandModeEnabled(false);
			return;
		}
		if (e.key === "Enter" && !e.shiftKey && !(e as any).isComposing) {
			e.preventDefault();
			sendChat();
			return;
		}
		if (e.key === "ArrowUp" && chatInput.selectionStart === 0 && !e.shiftKey) {
			e.preventDefault();
			handleHistoryUp();
			return;
		}
		if (e.key === "ArrowDown" && chatInput.selectionStart === chatInput.value.length && !e.shiftKey) {
			e.preventDefault();
			handleHistoryDown();
		}
	});
	S.chatSendBtn?.addEventListener("click", sendChat);
}

function initializeChatControls(): void {
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
	S.setProjectCombo(S.$("projectCombo"));
	S.setProjectComboBtn(S.$("projectComboBtn"));
	S.setProjectComboLabel(S.$("projectComboLabel"));
	S.setProjectDropdown(S.$("projectDropdown"));
	S.setProjectDropdownList(S.$("projectDropdownList"));
	bindProjectComboEvents();
	fetchProjects();
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

function bindContextModals(): {
	debugModal: HTMLElement | null;
	fullContextModal: HTMLElement | null;
	closeDebugModal: (() => void) | null;
	closeFullContextModal: (() => void) | null;
} {
	const debugModal = S.$("debugModal") as HTMLElement | null;
	const debugCloseBtn = S.$("debugModalCloseBtn") as HTMLElement | null;
	let closeDebugModal: (() => void) | null = null;
	if (debugModal) {
		closeDebugModal = () => setDebugModalOpen(false);
		if (debugCloseBtn) debugCloseBtn.addEventListener("click", closeDebugModal);
		debugModal.addEventListener("click", (e: MouseEvent) => {
			if (e.target === debugModal) closeDebugModal?.();
		});
	}
	const fullContextModal = S.$("fullContextModal") as HTMLElement | null;
	const fcCloseBtn = S.$("fullContextModalCloseBtn") as HTMLElement | null;
	let closeFullContextModal: (() => void) | null = null;
	if (fullContextModal) {
		closeFullContextModal = () => setFullContextModalOpen(false);
		if (fcCloseBtn) fcCloseBtn.addEventListener("click", closeFullContextModal);
		fullContextModal.addEventListener("click", (e: MouseEvent) => {
			if (e.target === fullContextModal) closeFullContextModal?.();
		});
	}
	contextModalsKeydownHandler = (e: KeyboardEvent): void => {
		if (e.key !== "Escape") return;
		if (fullContextModal && !fullContextModal.classList.contains("hidden")) {
			closeFullContextModal?.();
			return;
		}
		if (debugModal && !debugModal.classList.contains("hidden")) {
			closeDebugModal?.();
		}
	};
	document.addEventListener("keydown", contextModalsKeydownHandler);
	return { debugModal, fullContextModal, closeDebugModal, closeFullContextModal };
}

function syncModelComboLabel(): void {
	if (!(S.models.length > 0 && S.modelComboLabel)) return;
	const models = S.models as Array<{ id: string; displayName?: string }>;
	const found = models.find((m) => m.id === S.selectedModelId);
	if (found) {
		S.modelComboLabel.textContent = found.displayName || found.id;
		return;
	}
	if (models[0]) S.modelComboLabel.textContent = models[0].displayName || models[0].id;
}

function resolveInitialSessionKey(sessionKeyFromUrl: string | null): string {
	if (sessionKeyFromUrl) return sessionKeyFromUrl;
	const sk = localStorage.getItem("moltis-session") || "main";
	history.replaceState(null, "", sessionPath(sk));
	return sk;
}

function startInitialChatSession(sessionKey: string): void {
	if (!S.connected) return;
	(S.chatSendBtn as HTMLButtonElement).disabled = false;
	switchSession(sessionKey);
}

function initializeChatMediaDrop(): void {
	if (window.innerWidth < 768) return;
	const inputArea = S.chatInput?.closest(".px-4.py-3");
	initMediaDrop(S.chatMsgBox!, inputArea as HTMLElement);
}

// Safe: static hardcoded HTML template string — no user input is interpolated.
// This is a compile-time constant defined in the original JS source.
const chatPageHTML =
	'<div style="position:absolute;inset:0;display:grid;grid-template-rows:auto auto 1fr auto auto auto;overflow:hidden">' +
	'<div class="chat-toolbar h-12 px-4 border-b border-[var(--border)] bg-[var(--surface)] flex items-center gap-2" style="grid-row:1;">' +
	'<div id="modelCombo" class="model-combo"><button id="modelComboBtn" class="model-combo-btn" type="button"><span id="modelComboLabel">loading\u2026</span><span class="icon icon-sm icon-chevron-down model-combo-chevron"></span></button><div id="modelDropdown" class="model-dropdown hidden"><input id="modelSearchInput" type="text" placeholder="Search models\u2026" class="model-search-input" autocomplete="off" /><div id="modelDropdownList" class="model-dropdown-list"></div></div></div>' +
	'<div id="reasoningCombo" class="model-combo hidden"><button id="reasoningComboBtn" class="model-combo-btn" type="button" title="Reasoning effort"><span class="icon icon-sm icon-brain" style="flex-shrink:0;"></span><span id="reasoningComboLabel">Off</span><span class="icon icon-sm icon-chevron-down model-combo-chevron"></span></button><div id="reasoningDropdown" class="model-dropdown hidden"><div id="reasoningDropdownList" class="model-dropdown-list"></div></div></div>' +
	'<div id="nodeCombo" class="model-combo hidden"><button id="nodeComboBtn" class="model-combo-btn" type="button"><span class="icon icon-sm icon-server" style="flex-shrink:0;"></span><span id="nodeComboLabel">Local</span><span class="icon icon-sm icon-chevron-down model-combo-chevron"></span></button><div id="nodeDropdown" class="model-dropdown hidden" tabindex="-1"><div id="nodeDropdownList" class="model-dropdown-list"></div></div></div>' +
	'<div id="projectCombo" class="model-combo hidden"><button id="projectComboBtn" class="model-combo-btn" type="button"><span class="icon icon-sm icon-folder" style="flex-shrink:0;"></span><span id="projectComboLabel">No project</span><span class="icon icon-sm icon-chevron-down model-combo-chevron"></span></button><div id="projectDropdown" class="model-dropdown hidden"><div id="projectDropdownList" class="model-dropdown-list"></div></div></div>' +
	'<div id="sessionNameMount" class="ml-auto flex items-center min-w-0"></div>' +
	'<div id="sessionHeaderToolbarMount" class="flex items-center gap-1.5"></div>' +
	'<button id="sandboxToggle" class="sandbox-toggle text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1" title="Toggle sandbox mode"><span class="icon icon-md icon-lock shrink-0"></span><span id="sandboxLabel">sandboxed</span></button>' +
	'<div class="chat-badge-desktop-only" style="position:relative;display:inline-block"><button id="sandboxImageBtn" class="text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Sandbox image"><span class="icon icon-md icon-cube shrink-0"></span><span id="sandboxImageLabel" class="max-w-[120px] truncate">ubuntu:25.10</span></button><div id="sandboxImageDropdown" class="hidden" style="position:absolute;top:100%;left:0;z-index:50;margin-top:4px;min-width:200px;max-height:300px;overflow-y:auto;background:var(--surface);border:1px solid var(--border);border-radius:8px;box-shadow:0 4px 12px rgba(0,0,0,.15);"></div></div>' +
	'<button id="mcpToggleBtn" class="chat-badge-desktop-only text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1" title="Toggle MCP tools for this session"><span class="icon icon-md icon-link shrink-0"></span><span id="mcpToggleLabel">MCP</span></button>' +
	'<button id="debugPanelBtn" class="chat-badge-desktop-only text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Show context debug info"><span class="icon icon-md icon-wrench shrink-0"></span><span id="debugPanelLabel">Debug</span></button>' +
	'<button id="fullContextBtn" class="chat-badge-desktop-only text-xs border border-[var(--border)] px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent font-[var(--font-body)] inline-flex items-center gap-1 text-[var(--muted)]" title="Show full LLM context (system prompt + history)"><span class="icon icon-md icon-document shrink-0"></span><span id="fullContextLabel">Context</span></button>' +
	'<div id="sessionActionsMount" class="flex items-center gap-1.5"></div></div>' +
	'<div id="debugModal" class="provider-modal-backdrop hidden"><div class="provider-modal" style="width:min(980px,96vw);max-width:96vw;max-height:88vh;"><div class="provider-modal-header"><div class="provider-item-name">Debug context</div><button id="debugModalCloseBtn" type="button" class="provider-btn provider-btn-secondary provider-btn-sm">Close</button></div><div class="provider-modal-body" style="padding:0;overflow:hidden;"><div id="debugPanel" class="px-4 py-3 overflow-y-auto" style="max-height:72vh;"></div></div></div></div>' +
	'<div id="fullContextModal" class="provider-modal-backdrop hidden"><div class="provider-modal" style="width:min(1080px,96vw);max-width:96vw;max-height:88vh;"><div class="provider-modal-header"><div class="provider-item-name">Full context</div><button id="fullContextModalCloseBtn" type="button" class="provider-btn provider-btn-secondary provider-btn-sm">Close</button></div><div class="provider-modal-body" style="padding:0;overflow:hidden;"><div id="fullContextPanel" class="px-4 py-3 overflow-y-auto" style="max-height:72vh;"></div></div></div></div>' +
	'<div class="p-4 flex flex-col gap-2" id="messages" style="grid-row:3;overflow-y:auto;min-height:0"></div>' +
	'<div id="queuedMessages" class="queued-tray hidden" style="grid-row:4;"></div>' +
	'<div id="tokenBar" class="token-bar" style="grid-row:5;"></div>' +
	'<div class="chat-input-row px-4 py-3 border-t border-[var(--border)] bg-[var(--surface)] flex gap-2 items-end" style="grid-row:6;"><span id="chatCommandPrompt" class="chat-command-prompt chat-command-prompt-hidden" title="Command prompt symbol" aria-hidden="true">$</span><textarea id="chatInput" placeholder="Type a message..." rows="1" enterkeyhint="send" class="flex-1 bg-[var(--surface2)] border border-[var(--border)] text-[var(--text)] px-3 py-2 rounded-lg text-sm resize-none min-h-[40px] max-h-[120px] leading-relaxed focus:outline-none focus:border-[var(--border-strong)] focus:ring-1 focus:ring-[var(--accent-subtle)] transition-colors font-[var(--font-body)]"></textarea><button id="micBtn" disabled title="Click to start recording" class="mic-btn min-h-[40px] px-3 bg-[var(--surface2)] border border-[var(--border)] rounded-lg text-[var(--muted)] cursor-pointer disabled:opacity-40 disabled:cursor-default transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text)]"><span class="icon icon-lg icon-microphone"></span></button><button id="vadBtn" disabled title="Conversation mode (VAD)" class="vad-btn min-h-[40px] px-3 bg-[var(--surface2)] border border-[var(--border)] rounded-lg text-[var(--muted)] cursor-pointer disabled:opacity-40 disabled:cursor-default transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text)]"><span class="icon icon-lg icon-waveform"></span></button><button id="sendBtn" disabled class="provider-btn min-h-[40px] disabled:opacity-40 disabled:cursor-default">Send</button></div></div>';

// ── Page registration ────────────────────────────────────────

import { updateCommandInputUI } from "../chat-ui";

let chatScrollHandler: (() => void) | null = null;

registerPrefix(
	routes.chats!,
	function initChat(container: HTMLElement, sessionKeyFromUrl?: string | null) {
		container.style.cssText = "position:relative";
		// Safe: chatPageHTML is a static hardcoded template with no user input.
		// This is a compile-time constant defined above -- no dynamic or user data.
		container.innerHTML = chatPageHTML;

		S.setChatMsgBox(S.$("messages"));
		S.setChatInput(S.$("chatInput"));
		S.setChatSendBtn(S.$("sendBtn"));
		updateCommandInputUI();
		initializeChatControls();

		// Wire sub-module callbacks
		setSendChatFn(sendChat);
		setMaybeRefreshFullContextFn(maybeRefreshFullContext);

		mountSessionHeaderControls();

		const mcpToggle = S.$("mcpToggleBtn");
		if (mcpToggle) mcpToggle.addEventListener("click", toggleMcp);
		updateMcpToggleUI(true);

		bindContextModals();

		const debugBtn = S.$("debugPanelBtn");
		if (debugBtn) debugBtn.addEventListener("click", toggleDebugPanel);
		S.$("fullContextBtn")?.addEventListener("click", toggleFullContextPanel);

		syncModelComboLabel();
		const sessionKey = resolveInitialSessionKey(sessionKeyFromUrl ?? null);
		startInitialChatSession(sessionKey);
		bindChatComposer();
		S.chatMsgBox?.addEventListener("copy", handleChatCopy);

		// Smart auto-scroll: detect when user scrolls back to bottom
		chatScrollHandler = () => {
			if (isChatAtBottom()) hideNewContentIndicator();
		};
		S.chatMsgBox?.addEventListener("scroll", chatScrollHandler, { passive: true });

		initVoiceInput(S.$("micBtn") as HTMLButtonElement | null);
		initVadButton(S.$("vadBtn") as HTMLButtonElement | null);
		initializeChatMediaDrop();
		S.chatInput?.focus();
	},
	function teardownChat() {
		if (chatScrollHandler) {
			S.chatMsgBox?.removeEventListener("scroll", chatScrollHandler);
			chatScrollHandler = null;
		}
		S.chatMsgBox?.removeEventListener("copy", handleChatCopy);
		teardownVoiceInput();
		teardownMediaDrop();
		unbindReasoningToggle();
		unbindNodeEvents();
		slashHideMenu();
		if (contextModalsKeydownHandler) {
			document.removeEventListener("keydown", contextModalsKeydownHandler);
			contextModalsKeydownHandler = null;
		}
		const m0 = S.$("sessionNameMount");
		if (m0) render(null, m0);
		const m1 = S.$("sessionHeaderToolbarMount");
		if (m1) render(null, m1);
		const m2 = S.$("sessionActionsMount");
		if (m2) render(null, m2);
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
		S.setProjectCombo(null);
		S.setProjectComboBtn(null);
		S.setProjectComboLabel(null);
		S.setProjectDropdown(null);
		S.setProjectDropdownList(null);
	},
);
