// ── Slash command UI ─────────────────────────────────────────

import { chatAddMsg, updateCommandInputUI } from "../../chat-ui";
import { renderMarkdown, sendRpc } from "../../helpers";
import { clearActiveSession, fetchSessions, switchSession } from "../../sessions";
import * as S from "../../state";
import { type ContextData, renderContextCard } from "./context-card";

// ── Types ────────────────────────────────────────────────────

/** Known slash command names — adding a name here requires a handler in `slashHandlers`. */
type SlashCommandName =
	| "btw"
	| "clear"
	| "compact"
	| "context"
	| "fast"
	| "fork"
	| "insights"
	| "mode"
	| "new"
	| "reset"
	| "rollback"
	| "sh";

export interface SlashCommand {
	name: SlashCommandName;
	description: string;
}

/** Handler function for a slash command. */
type SlashHandler = (args: string) => void;

export interface ParsedSlash {
	name: string;
	args: string;
}

interface UnknownRecord {
	[key: string]: unknown;
}

interface ModePayload {
	id: string;
	name: string;
	description: string;
	prompt: string;
}

interface InsightsApiResponse {
	days: number;
	completions: number;
	input_tokens: number;
	output_tokens: number;
	total_tokens: number;
	errors: number;
	tool_executions: number;
	tool_errors: number;
	by_provider: Record<string, { input_tokens: number; output_tokens: number; completions: number }>;
	data_points: number;
	span_hours: number;
}

function fmtNum(n: number): string {
	if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
	if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
	return n.toLocaleString();
}

interface ModesListPayload {
	modes: ModePayload[];
}

// ── Slash commands list ─────────────────────────────────────

export const slashCommands: SlashCommand[] = [
	{ name: "btw", description: "Quick side question (no tools, not persisted)" },
	{ name: "clear", description: "Clear conversation history" },
	{ name: "compact", description: "Summarize conversation to save tokens" },
	{ name: "context", description: "Show session context and project info" },
	{ name: "fast", description: "Toggle fast/priority mode" },
	{ name: "fork", description: "Fork this session into a new branch" },
	{ name: "insights", description: "Show usage analytics (tokens, providers)" },
	{ name: "mode", description: "Switch session mode (/mode none to clear)" },
	{ name: "new", description: "Start a new session" },
	{ name: "reset", description: "Clear conversation history" },
	{ name: "rollback", description: "List or restore file checkpoints" },
	{ name: "sh", description: "Enter command mode (/sh off or Esc to exit)" },
];

let slashMenuEl: HTMLDivElement | null = null;
let slashMenuIdx = 0;
let slashMenuItems: SlashCommand[] = [];
let sendChatFn: (() => void) | null = null;

/** Called by ChatPage to wire up the sendChat callback for slash menu selection. */
export function setSendChatFn(fn: () => void): void {
	sendChatFn = fn;
}

// ── Style injection ─────────────────────────────────────────

export function slashInjectStyles(): void {
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

// ── Menu display ────────────────────────────────────────────

export function slashShowMenu(filter: string): void {
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
		slashMenuEl?.appendChild(item);
	});

	const inputWrap = S.chatInput?.parentElement;
	if (inputWrap && !slashMenuEl.parentElement) {
		inputWrap.classList.add("relative");
		inputWrap.appendChild(slashMenuEl);
	}
}

export function slashHideMenu(): void {
	if (slashMenuEl?.parentElement) {
		slashMenuEl.parentElement.removeChild(slashMenuEl);
	}
	slashMenuItems = [];
	slashMenuIdx = 0;
}

export function slashSelectItem(idx: number): void {
	if (!slashMenuItems[idx]) return;
	(S.chatInput as HTMLTextAreaElement).value = `/${slashMenuItems[idx].name}`;
	slashHideMenu();
	sendChatFn?.();
}

export function slashHandleInput(): void {
	const val = (S.chatInput as HTMLTextAreaElement).value;
	if (val.indexOf("/") === 0 && val.indexOf(" ") === -1) {
		slashShowMenu(val);
	} else {
		slashHideMenu();
	}
}

export function slashHandleKeydown(e: KeyboardEvent): boolean {
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

export function slashUpdateActive(): void {
	if (!slashMenuEl) return;
	const items = slashMenuEl.querySelectorAll(".slash-menu-item");
	items.forEach((el, i) => {
		el.classList.toggle("active", i === slashMenuIdx);
	});
}

export function parseSlashCommand(text: string): ParsedSlash | null {
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

export function shouldHandleSlashLocally(cmdName: string, args: string): boolean {
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

function isRecord(value: unknown): value is UnknownRecord {
	return typeof value === "object" && value !== null;
}

function parseMode(value: unknown): ModePayload | null {
	if (!isRecord(value)) return null;
	const id = typeof value.id === "string" ? value.id : "";
	if (!id) return null;
	return {
		id,
		name: typeof value.name === "string" && value.name.trim() ? value.name : id,
		description: typeof value.description === "string" ? value.description : "",
		prompt: typeof value.prompt === "string" ? value.prompt : "",
	};
}

function parseModesListPayload(value: unknown): ModesListPayload {
	if (!(isRecord(value) && Array.isArray(value.modes))) return { modes: [] };
	return { modes: value.modes.map(parseMode).filter((mode): mode is ModePayload => mode !== null) };
}

function formatModeList(modes: ModePayload[]): string {
	if (modes.length === 0) return "No modes are configured.";
	const lines = modes.map((mode, index) => {
		const description = mode.description ? ` - ${mode.description}` : "";
		return `${index + 1}. ${mode.name} [${mode.id}]${description}`;
	});
	lines.push("", "Use `/mode N`, `/mode <id>`, or `/mode none`.");
	return lines.join("\n");
}

function findMode(modes: ModePayload[], args: string): ModePayload | null {
	const normalized = args.trim().toLowerCase();
	const number = Number.parseInt(normalized, 10);
	if (Number.isInteger(number) && number > 0 && String(number) === normalized) {
		return modes[number - 1] || null;
	}
	return modes.find((mode) => mode.id.toLowerCase() === normalized || mode.name.toLowerCase() === normalized) || null;
}

function handleModeCommand(cmdArgs: string): void {
	const args = cmdArgs.trim();
	chatAddMsg("system", "Loading modes...");
	sendRpc("modes.list", {}).then((listRes) => {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		if (!listRes?.ok) {
			chatAddMsg("error", listRes?.error?.message || "Failed to load modes");
			return;
		}
		const modes = parseModesListPayload(listRes.payload).modes;
		if (!args) {
			chatAddMsg("system", renderMarkdown(formatModeList(modes)), true);
			return;
		}
		const normalized = args.toLowerCase();
		if (["none", "off", "clear", "default", "reset"].includes(normalized)) {
			sendRpc("modes.set_session", { session_key: S.activeSessionKey, mode_id: null }).then((setRes) => {
				if (!setRes?.ok) {
					chatAddMsg("error", setRes?.error?.message || "Failed to clear mode");
					return;
				}
				fetchSessions();
				chatAddMsg("system", renderMarkdown("**Mode:** cleared"), true);
			});
			return;
		}
		const selected = findMode(modes, args);
		if (!selected) {
			chatAddMsg("error", `Unknown mode: ${args}`);
			return;
		}
		sendRpc("modes.set_session", { session_key: S.activeSessionKey, mode_id: selected.id }).then((setRes) => {
			if (!setRes?.ok) {
				chatAddMsg("error", setRes?.error?.message || "Failed to set mode");
				return;
			}
			fetchSessions();
			chatAddMsg("system", renderMarkdown(`**Mode:** ${selected.name}`), true);
		});
	});
}

/**
 * Handler map — every `SlashCommandName` must have an entry here.
 * TypeScript will error at `satisfies` if a command is missing.
 */
const slashHandlers: Record<SlashCommandName, SlashHandler> = {
	clear: () => clearActiveSession(),

	compact: () => {
		chatAddMsg("system", "Compacting conversation\u2026");
		sendRpc("chat.compact", {}).then((res) => {
			if (res.ok) switchSession(S.activeSessionKey);
			else chatAddMsg("error", res.error?.message || "Compact failed");
		});
	},

	context: () => {
		chatAddMsg("system", "Loading context\u2026");
		sendRpc("chat.context", {}).then((res) => {
			if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
			if (res.ok && res.payload) {
				try {
					renderContextCard(res.payload as ContextData);
				} catch (err: unknown) {
					const message = err instanceof Error ? err.message : "Unknown render error";
					chatAddMsg("error", `Render error: ${message}`);
				}
			} else chatAddMsg("error", res.error?.message || "Context failed");
		});
	},

	mode: (args) => handleModeCommand(args),

	new: () => switchSession(`session:${crypto.randomUUID()}`),

	reset: () => {
		chatAddMsg("system", "Resetting session\u2026");
		sendRpc("sessions.reset", { key: S.activeSessionKey }).then((res) => {
			if (res.ok) switchSession(S.activeSessionKey);
			else chatAddMsg("error", res.error?.message || "Reset failed");
		});
	},

	sh: (args) => {
		const normalized = (args || "").toLowerCase();
		if (normalized === "off" || normalized === "exit") {
			setCommandMode(false);
			chatAddMsg("system", renderMarkdown("**Command:** mode disabled"), true);
			return;
		}
		setCommandMode(true);
		chatAddMsg(
			"system",
			renderMarkdown(`**Command:** mode enabled (${commandModeSummary()}) \u00b7 exit with /sh off or Esc`),
			true,
		);
	},

	insights: (args) => {
		const days = args.trim() || "30";
		chatAddMsg("system", `Loading insights for last ${days} days\u2026`);
		fetch(`/api/metrics/insights?days=${days}`)
			.then((resp) => (resp.ok ? resp.json() : Promise.reject(new Error(`HTTP ${resp.status}`))))
			.then((data: InsightsApiResponse) => {
				if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
				const lines: string[] = [];
				lines.push(`**Insights** \u2014 last ${data.days} days\n`);
				lines.push("| Metric | Value |");
				lines.push("|--------|-------|");
				lines.push(`| LLM completions | ${fmtNum(data.completions)} |`);
				lines.push(`| Total tokens | ${fmtNum(data.total_tokens)} |`);
				lines.push(`| Input tokens | ${fmtNum(data.input_tokens)} |`);
				lines.push(`| Output tokens | ${fmtNum(data.output_tokens)} |`);
				lines.push(`| LLM errors | ${fmtNum(data.errors)} |`);
				lines.push(`| Tool executions | ${fmtNum(data.tool_executions)} |`);
				if (data.span_hours > 0) {
					lines.push(`| Completions/hour | ${(data.completions / data.span_hours).toFixed(1)} |`);
				}
				if (data.by_provider && Object.keys(data.by_provider).length > 0) {
					lines.push("\n**By provider:**\n");
					lines.push("| Provider | Completions | Tokens |");
					lines.push("|----------|-------------|--------|");
					const sorted = Object.entries(data.by_provider).sort(
						(a, b) => b[1].input_tokens + b[1].output_tokens - (a[1].input_tokens + a[1].output_tokens),
					);
					for (const [name, stats] of sorted) {
						lines.push(
							`| ${name} | ${fmtNum(stats.completions)} | ${fmtNum(stats.input_tokens + stats.output_tokens)} |`,
						);
					}
				}
				lines.push(`\n*${fmtNum(data.data_points)} data points over ${data.span_hours.toFixed(1)} hours*`);
				chatAddMsg("system", renderMarkdown(lines.join("\n")), true);
			})
			.catch((err: Error) => chatAddMsg("error", `Insights failed: ${err.message}`));
	},

	btw: (args) => {
		if (!args.trim()) {
			chatAddMsg("error", "Usage: /btw <question>");
			return;
		}
		chatAddMsg("system", "Thinking\u2026");
		sendRpc("chat.send_sync", { text: args, _ephemeral: true }).then((res) => {
			if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
			if (res.ok && res.payload) {
				const text = typeof res.payload === "string" ? res.payload : (res.payload as UnknownRecord).text;
				chatAddMsg("system", renderMarkdown(String(text || "(no response)")), true);
			} else chatAddMsg("error", res.error?.message || "/btw failed");
		});
	},

	fast: (args) => {
		const arg = args.trim().toLowerCase();
		chatAddMsg("system", `Fast mode: ${arg || "toggle"}\u2026`);
		sendRpc("chat.send_sync", { text: `/fast ${arg}`.trim() }).then((res) => {
			if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
			if (res.ok && res.payload) {
				const text = typeof res.payload === "string" ? res.payload : (res.payload as UnknownRecord).text;
				chatAddMsg("system", String(text || "Done"));
			} else chatAddMsg("error", res.error?.message || "/fast failed");
		});
	},

	fork: (args) => {
		chatAddMsg("system", "Forking session\u2026");
		sendRpc("sessions.fork", { key: S.activeSessionKey, label: args.trim() || undefined }).then((res) => {
			if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
			if (res.ok && res.payload) {
				const key = (res.payload as UnknownRecord).sessionKey as string;
				const label = (res.payload as UnknownRecord).label as string;
				chatAddMsg("system", `Forked into: ${label || key}. Use the session list to switch.`);
				fetchSessions();
			} else chatAddMsg("error", res.error?.message || "Fork failed");
		});
	},

	rollback: () => {
		chatAddMsg(
			"system",
			renderMarkdown("Rollback is available via the `/rollback` channel command (Telegram, Discord, etc.) or the CLI."),
			true,
		);
	},
};

export function handleSlashCommand(cmdName: string, cmdArgs: string): void {
	const handler = slashHandlers[cmdName as SlashCommandName];
	if (handler) handler(cmdArgs);
}
