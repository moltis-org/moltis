// ── Session rendering: history messages, welcome card, session list ──

import {
	appendChannelFooter,
	appendReasoningDisclosure,
	chatAddMsg,
	chatAddMsgWithImages,
	highlightAndScroll,
	removeThinking,
	scrollChatToBottom,
	stripChannelPrefix,
	updateCommandInputUI,
	updateTokenBar,
} from "../chat-ui";
import { highlightCodeBlocks } from "../code-highlight";
import * as gon from "../gon";
import {
	formatAssistantTokenUsage,
	formatTokenSpeed,
	parseAgentsListPayload,
	renderAudioPlayer,
	renderDocument,
	renderMarkdown,
	renderScreenshot,
	sendRpc,
	tokenSpeedTone,
	toolCallSummary,
} from "../helpers";
import { attachMessageVoiceControl } from "../message-voice";
import { navigate } from "../router";
import { settingsPath } from "../routes";
import * as S from "../state";
import { modelStore } from "../stores/model-store";
import { sessionStore } from "../stores/session-store";
import type { HistoryMessage } from "../types";

import { computeHistoryTailIndex, ensureHistoryScrollBinding, syncHistoryState } from "./session-history";

// ── Types ────────────────────────────────────────────────────

export interface SearchContext {
	query: string;
	messageIndex: number;
}

interface ToolResultMsg extends HistoryMessage {
	tool_name?: string;
	arguments?: unknown;
	success?: boolean;
	result?: {
		stdout?: string;
		stderr?: string;
		exit_code?: number;
		screenshot?: string;
		document_ref?: string;
		filename?: string;
		mime_type?: string;
		size_bytes?: number;
	};
	error?: string;
	reasoning?: string;
}

interface AssistantMsg extends HistoryMessage {
	content?: string;
	model?: string;
	provider?: string;
	inputTokens?: number;
	outputTokens?: number;
	cacheReadTokens?: number;
	durationMs?: number;
	reasoning?: string;
	audio?: string;
	run_id?: string;
	historyIndex?: number;
	requestInputTokens?: number;
}

interface UserMsg extends Omit<HistoryMessage, "content"> {
	content?: string | unknown[];
	channel?: {
		channel_type?: string;
		username?: string;
		sender_name?: string;
		message_kind?: string;
	};
	audio?: string;
}

interface AgentInfo {
	id?: string;
	name?: string;
	emoji?: string;
}

/** History message with an optional seq field, used for resuming chat sequence counters. */
interface SeqHistoryMessage extends HistoryMessage {
	seq?: number;
	created_at?: number;
}

/** Token usage counters returned by chat.context RPC. */
interface TokenUsage {
	contextWindow?: number;
	inputTokens?: number;
	outputTokens?: number;
	estimatedNextInputTokens?: number;
	currentInputTokens?: number;
}

/** Execution environment info returned by chat.context RPC. */
interface ExecutionInfo {
	mode?: string;
	isRoot?: boolean;
	hostIsRoot?: boolean;
}

/** Payload returned by the chat.context RPC. */
interface ChatContextPayload {
	tokenUsage?: TokenUsage;
	supportsTools?: boolean;
	execution?: ExecutionInfo;
}

// ── Multimodal parsing ───────────────────────────────────────

/** Extract text and images from a multimodal content array. */
function parseMultimodalContent(blocks: unknown[]): { text: string; images: { dataUrl: string; name: string }[] } {
	let text = "";
	const images: { dataUrl: string; name: string }[] = [];
	for (const block of blocks as Array<{ type?: string; text?: string; image_url?: { url?: string } }>) {
		if (block.type === "text") {
			text = block.text || "";
		} else if (block.type === "image_url" && block.image_url?.url) {
			images.push({ dataUrl: block.image_url.url, name: "image" });
		}
	}
	return { text, images };
}

// ── History message renderers ────────────────────────────────

function renderHistoryUserMessage(msg: UserMsg): HTMLElement | null {
	let text = "";
	let images: { dataUrl: string; name: string }[] = [];
	if (Array.isArray(msg.content)) {
		const parsed = parseMultimodalContent(msg.content);
		text = msg.channel ? stripChannelPrefix(parsed.text) : parsed.text;
		images = parsed.images;
	} else {
		text = msg.channel ? stripChannelPrefix((msg.content as string) || "") : (msg.content as string) || "";
	}

	let el: HTMLElement | null;
	if (msg.audio) {
		el = chatAddMsg("user", "", true);
		if (el) {
			const filename = msg.audio.split("/").pop() || "";
			const audioSrc = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			renderAudioPlayer(el, audioSrc);
			if (text) {
				const textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				// Safe: renderMarkdown escapes user input before formatting tags.
				textWrap.insertAdjacentHTML("beforeend", renderMarkdown(text));
				el.appendChild(textWrap);
			}
			if (images.length > 0) {
				const thumbRow = document.createElement("div");
				thumbRow.className = "msg-image-row";
				for (const img of images) {
					const thumb = document.createElement("img");
					thumb.className = "msg-image-thumb";
					thumb.src = img.dataUrl;
					thumb.alt = img.name;
					thumbRow.appendChild(thumb);
				}
				el.appendChild(thumbRow);
			}
		}
	} else if (images.length > 0) {
		el = chatAddMsgWithImages("user", text ? renderMarkdown(text) : "", images);
	} else {
		el = chatAddMsg("user", renderMarkdown(text), true);
	}
	if (el && msg.channel) appendChannelFooter(el, msg.channel);
	return el;
}

function createModelFooter(msg: AssistantMsg): HTMLDivElement {
	const ft = document.createElement("div");
	ft.className = "msg-model-footer";
	let ftText = msg.provider ? `${msg.provider} / ${msg.model}` : msg.model || "";
	if (msg.inputTokens || msg.outputTokens) {
		ftText += ` \u00b7 ${formatAssistantTokenUsage(msg.inputTokens || 0, msg.outputTokens || 0, msg.cacheReadTokens || 0)}`;
	}
	const textSpan = document.createElement("span");
	textSpan.textContent = ftText;
	ft.appendChild(textSpan);

	const speedLabel = formatTokenSpeed(msg.outputTokens || 0, msg.durationMs || 0);
	if (speedLabel) {
		const speed = document.createElement("span");
		speed.className = "msg-token-speed";
		const tone = tokenSpeedTone(msg.outputTokens || 0, msg.durationMs || 0);
		if (tone) speed.classList.add(`msg-token-speed-${tone}`);
		speed.textContent = ` \u00b7 ${speedLabel}`;
		ft.appendChild(speed);
	}
	return ft;
}

function renderHistoryAssistantMessage(msg: AssistantMsg): HTMLElement | null {
	let el: HTMLElement | null;
	if (msg.audio) {
		el = chatAddMsg("assistant", "", true);
		if (el) {
			const filename = msg.audio.split("/").pop() || "";
			const audioSrc = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/media/${encodeURIComponent(filename)}`;
			renderAudioPlayer(el, audioSrc);
			if (msg.content) {
				const textWrap = document.createElement("div");
				textWrap.className = "mt-2";
				textWrap.insertAdjacentHTML("beforeend", renderMarkdown(msg.content));
				el.appendChild(textWrap);
			}
			if (msg.reasoning) {
				appendReasoningDisclosure(el, msg.reasoning);
			}
		}
	} else {
		el = chatAddMsg("assistant", renderMarkdown(msg.content || ""), true);
		if (el && msg.reasoning) {
			appendReasoningDisclosure(el, msg.reasoning);
		}
	}
	if (el && msg.model) {
		const footer = createModelFooter(msg);
		el.appendChild(footer);
		void attachMessageVoiceControl({
			messageEl: el,
			footerEl: footer,
			sessionKey: S.activeSessionKey,
			text: msg.content || "",
			runId: msg.run_id || undefined,
			messageIndex: msg.historyIndex,
			audioPath: msg.audio || undefined,
			audioWarning: undefined,
			forceAction: false,
			autoplayOnGenerate: true,
		});
	}
	if (msg.inputTokens || msg.outputTokens) {
		S.sessionTokens.input += msg.inputTokens || 0;
		S.sessionTokens.output += msg.outputTokens || 0;
	}
	if (msg.requestInputTokens !== undefined && msg.requestInputTokens !== null) {
		S.setSessionCurrentInputTokens(msg.requestInputTokens || 0);
	} else if (msg.inputTokens || msg.outputTokens) {
		S.setSessionCurrentInputTokens(msg.inputTokens || 0);
	}
	return el;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Sequential result field rendering
function renderHistoryToolResult(msg: ToolResultMsg): HTMLElement {
	const tpl = S.$<HTMLTemplateElement>("tpl-exec-card")!;
	const frag = tpl.content.cloneNode(true) as DocumentFragment;
	const card = frag.firstElementChild as HTMLElement;

	const statusEl = card.querySelector(".exec-status");
	if (statusEl) statusEl.remove();

	const cmd = toolCallSummary(msg.tool_name, msg.arguments as Parameters<typeof toolCallSummary>[1]);
	(card.querySelector("[data-cmd]") as HTMLElement).textContent = ` ${cmd}`;

	card.className = `msg exec-card ${msg.success ? "exec-ok" : "exec-err"}`;

	if (msg.result) {
		const out = (msg.result.stdout || "").replace(/\n+$/, "");
		if (out) {
			const outEl = document.createElement("pre");
			outEl.className = "exec-output";
			outEl.textContent = out;
			card.appendChild(outEl);
		}
		const stderrText = (msg.result.stderr || "").replace(/\n+$/, "");
		if (stderrText) {
			const errEl = document.createElement("pre");
			errEl.className = "exec-output exec-stderr";
			errEl.textContent = stderrText;
			card.appendChild(errEl);
		}
		if (msg.result.exit_code !== undefined && msg.result.exit_code !== 0) {
			const codeEl = document.createElement("div");
			codeEl.className = "exec-exit";
			codeEl.textContent = `exit ${msg.result.exit_code}`;
			card.appendChild(codeEl);
		}
		if (msg.result.screenshot && !msg.result.screenshot.startsWith("data:")) {
			const filename = msg.result.screenshot.split("/").pop() || "";
			const sessionKey = S.activeSessionKey || "main";
			const mediaSrc = `/api/sessions/${encodeURIComponent(sessionKey)}/media/${encodeURIComponent(filename)}`;
			renderScreenshot(card, mediaSrc);
		}
		if (msg.result.document_ref) {
			const docStoredName = msg.result.document_ref.split("/").pop() || "";
			const docDisplayName = msg.result.filename || docStoredName;
			const docSessionKey = S.activeSessionKey || "main";
			const docMediaSrc = `/api/sessions/${encodeURIComponent(docSessionKey)}/media/${encodeURIComponent(docStoredName)}`;
			renderDocument(card, docMediaSrc, docDisplayName, msg.result.mime_type, msg.result.size_bytes);
		}
	}

	if (!msg.success && msg.error) {
		const errMsg = document.createElement("div");
		errMsg.className = "exec-error-detail";
		errMsg.textContent = msg.error;
		card.appendChild(errMsg);
	}

	if (msg.reasoning) {
		appendReasoningDisclosure(card, msg.reasoning);
	}

	if (S.chatMsgBox) S.chatMsgBox.appendChild(card);
	return card;
}

export function appendLastMessageTimestamp(epochMs: number): void {
	if (!S.chatMsgBox) return;
	const old = S.chatMsgBox.querySelector(".msg-footer-time");
	if (old) old.remove();
	const lastMsg = S.chatMsgBox.lastElementChild;
	if (!lastMsg || lastMsg.classList.contains("user")) return;
	let footer = lastMsg.querySelector(".msg-model-footer") as HTMLElement | null;
	if (!footer) {
		footer = document.createElement("div");
		footer.className = "msg-model-footer";
		lastMsg.appendChild(footer);
	}
	const timeEl = document.createElement("time");
	timeEl.className = "msg-footer-time";
	timeEl.setAttribute("data-epoch-ms", String(epochMs));
	timeEl.textContent = new Date(epochMs).toISOString();
	const wrap = document.createElement("span");
	wrap.className = "msg-footer-time";
	wrap.appendChild(document.createTextNode(" \u00b7 "));
	wrap.appendChild(timeEl);
	footer.appendChild(wrap);
}

function makeThinkingDots(): HTMLElement {
	const tpl = S.$<HTMLTemplateElement>("tpl-thinking-dots")!;
	return (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
}

export function postHistoryLoadActions(
	key: string,
	searchContext: SearchContext | null,
	msgEls: (HTMLElement | null)[],
	thinkingText: string | null,
	skipAutoScroll: boolean,
): void {
	sendRpc("chat.context", {}).then((ctxRes) => {
		if (ctxRes?.ok && ctxRes.payload) {
			const p = ctxRes.payload as ChatContextPayload;
			if (p.tokenUsage) {
				const tu = p.tokenUsage;
				S.setSessionContextWindow(tu.contextWindow || 0);
				S.setSessionTokens({
					input: tu.inputTokens || 0,
					output: tu.outputTokens || 0,
				});
				S.setSessionCurrentInputTokens(tu.estimatedNextInputTokens || tu.currentInputTokens || tu.inputTokens || 0);
			}
			S.setSessionToolsEnabled(p.supportsTools !== false);
			const execution = p.execution || {};
			const mode = execution.mode === "sandbox" ? "sandbox" : "host";
			const hostIsRoot = execution.hostIsRoot === true;
			let isRoot = execution.isRoot;
			if (typeof isRoot !== "boolean") {
				isRoot = mode === "sandbox" ? true : hostIsRoot;
			}
			S.setHostExecIsRoot(hostIsRoot);
			S.setSessionExecMode(mode);
			S.setSessionExecPromptSymbol(isRoot ? "#" : "$");
		}
		updateCommandInputUI();
		updateTokenBar();
	});
	updateTokenBar();

	if (!skipAutoScroll && searchContext?.query && S.chatMsgBox) {
		highlightAndScroll(msgEls, searchContext.messageIndex, searchContext.query);
	} else if (!skipAutoScroll) {
		scrollChatToBottom();
	}

	const session = sessionStore.getByKey(key);
	if (session?.replying.value && S.chatMsgBox) {
		removeThinking();
		const thinkEl = document.createElement("div");
		thinkEl.className = "msg assistant thinking";
		thinkEl.id = "thinkingIndicator";
		if (thinkingText) {
			const textEl = document.createElement("span");
			textEl.className = "thinking-text";
			textEl.textContent = thinkingText;
			thinkEl.appendChild(textEl);
		} else {
			thinkEl.appendChild(makeThinkingDots());
		}
		S.chatMsgBox.appendChild(thinkEl);
		if (!skipAutoScroll) scrollChatToBottom();
	}
}

/** No-op -- the Preact SessionHeader component auto-updates from signals. */
export function updateChatSessionHeader(): void {
	// Retained for backward compat call sites; Preact handles rendering.
}

export function renderWelcomeAgentPicker(
	card: HTMLElement,
	activeAgentId: string,
	onActiveAgentResolved: (agent: AgentInfo | null) => void,
): void {
	const container = card.querySelector("[data-welcome-agents]") as HTMLElement | null;
	if (!container) return;

	sendRpc("agents.list", {}).then((res) => {
		if (!card.isConnected) return;
		if (!res?.ok) {
			container.classList.add("hidden");
			return;
		}
		const parsed = parseAgentsListPayload(res.payload as Parameters<typeof parseAgentsListPayload>[0]);
		const agents = (parsed.agents || []) as AgentInfo[];
		const defaultId = (parsed.defaultId || "main") as string;
		const effectiveActive = activeAgentId || defaultId;

		container.textContent = "";
		container.classList.remove("hidden");
		container.classList.add("flex");

		let activeAgent: AgentInfo | null = null;
		for (const agent of agents) {
			if (!agent?.id) continue;
			if (agent.id === effectiveActive) activeAgent = agent;
			const chip = document.createElement("button");
			chip.type = "button";
			chip.className = agent.id === effectiveActive ? "provider-btn" : "provider-btn provider-btn-secondary";
			chip.style.fontSize = "0.7rem";
			chip.style.padding = "3px 8px";
			const labelPrefix = agent.emoji ? `${agent.emoji} ` : "";
			chip.textContent = `${labelPrefix}${agent.name || agent.id}`;
			chip.addEventListener("click", () => {
				const key = sessionStore.activeSessionKey.value || S.activeSessionKey || "main";
				sendRpc("agents.set_session", { session_key: key, agent_id: agent.id }).then((setRes) => {
					if (!setRes?.ok) return;
					const live = sessionStore.getByKey(key);
					if (live) {
						live.agent_id = agent.id || "";
						live.dataVersion.value++;
					}
					// Lazy import to avoid circular dependency with sessions.ts
					void import("../sessions").then(({ fetchSessions }) => fetchSessions());
					const welcome = S.chatMsgBox?.querySelector("#welcomeCard");
					if (welcome) {
						welcome.remove();
						showWelcomeCard();
					}
				});
			});
			container.appendChild(chip);
		}

		const hatchBtn = document.createElement("button");
		hatchBtn.type = "button";
		hatchBtn.className = "provider-btn provider-btn-secondary";
		hatchBtn.style.fontSize = "0.7rem";
		hatchBtn.style.padding = "3px 8px";
		hatchBtn.textContent = "\u{1F95A} Hatch a new agent";
		hatchBtn.addEventListener("click", () => {
			navigate(settingsPath("agents/new"));
		});
		container.appendChild(hatchBtn);

		onActiveAgentResolved(activeAgent);
	});
}

function showWelcomeCard(): void {
	if (!S.chatMsgBox) return;
	S.chatMsgBox.classList.add("chat-messages-empty");

	if (modelStore.models.value.length === 0) {
		const noProvTpl = S.$<HTMLTemplateElement>("tpl-no-providers-card");
		if (!noProvTpl) return;
		const noProvCard = (noProvTpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
		S.chatMsgBox.appendChild(noProvCard);
		return;
	}

	const tpl = S.$<HTMLTemplateElement>("tpl-welcome-card");
	if (!tpl) return;
	const card = (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
	const identity = gon.get("identity");
	const userName = identity?.user_name;
	const botName = identity?.name || "moltis";
	const botEmoji = identity?.emoji || "";

	const greetingEl = card.querySelector("[data-welcome-greeting]") as HTMLElement | null;
	if (greetingEl) greetingEl.textContent = userName ? `Hello, ${userName}!` : "Hello!";
	const emojiEl = card.querySelector("[data-welcome-emoji]") as HTMLElement | null;
	if (emojiEl) emojiEl.textContent = botEmoji;
	const nameEl = card.querySelector("[data-welcome-bot-name]") as HTMLElement | null;
	if (nameEl) nameEl.textContent = botName;
	const activeAgentId = sessionStore.activeSession.value?.agent_id || "main";
	renderWelcomeAgentPicker(card, activeAgentId, (activeAgent) => {
		if (!activeAgent) return;
		if (emojiEl) emojiEl.textContent = activeAgent.emoji || "";
		if (nameEl) nameEl.textContent = activeAgent.name || botName;
	});

	S.chatMsgBox.appendChild(card);
}

export function refreshWelcomeCardIfNeeded(): void {
	if (!S.chatMsgBox) return;
	const welcomeCard = S.chatMsgBox.querySelector("#welcomeCard");
	const noProvCard = S.chatMsgBox.querySelector("#noProvidersCard");
	const hasModels = modelStore.models.value.length > 0;

	if (hasModels && noProvCard) {
		noProvCard.remove();
		showWelcomeCard();
	} else if (!hasModels && welcomeCard) {
		welcomeCard.remove();
		showWelcomeCard();
	}
}

export function showSessionLoadIndicator(): void {
	if (!S.chatMsgBox) return;
	hideSessionLoadIndicator();
	const loading = document.createElement("div");
	loading.id = "sessionLoadIndicator";
	loading.className = "msg assistant thinking session-loading";
	loading.appendChild(makeThinkingDots());
	const label = document.createElement("span");
	label.className = "session-loading-label";
	label.textContent = "Loading session\u2026";
	loading.appendChild(label);
	S.chatMsgBox.appendChild(loading);
}

export function hideSessionLoadIndicator(): void {
	const loading = document.getElementById("sessionLoadIndicator");
	if (loading) loading.remove();
}

export function renderHistory(
	key: string,
	history: HistoryMessage[],
	searchContext: SearchContext | null,
	thinkingText: string | null,
	totalCountHint: number | null,
	skipAutoScroll: boolean,
): void {
	ensureHistoryScrollBinding();
	hideSessionLoadIndicator();
	if (S.chatMsgBox) {
		S.chatMsgBox.classList.remove("chat-messages-empty");
		S.chatMsgBox.textContent = "";
	}
	const msgEls: (HTMLElement | null)[] = [];
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionCurrentInputTokens(0);
	S.setChatBatchLoading(true);
	history.forEach((msg) => {
		if (msg.role === "user") {
			msgEls.push(renderHistoryUserMessage(msg as UserMsg));
		} else if (msg.role === "assistant") {
			msgEls.push(renderHistoryAssistantMessage(msg as AssistantMsg));
		} else if (msg.role === "notice") {
			msgEls.push(chatAddMsg("system", renderMarkdown(typeof msg.content === "string" ? msg.content : ""), true));
		} else if (msg.role === "tool_result") {
			msgEls.push(renderHistoryToolResult(msg as ToolResultMsg));
		} else {
			msgEls.push(null);
		}
	});
	S.setChatBatchLoading(false);
	if (S.chatMsgBox) highlightCodeBlocks(S.chatMsgBox);
	const historyTailIndex = computeHistoryTailIndex(history);
	syncHistoryState(key, history, historyTailIndex, totalCountHint);

	let maxSeq = 0;
	for (const hm of history) {
		if (hm.role === "user" && ((hm as SeqHistoryMessage).seq as number) > maxSeq) {
			maxSeq = (hm as SeqHistoryMessage).seq as number;
		}
	}
	S.setChatSeq(maxSeq);
	if (history.length === 0) {
		showWelcomeCard();
	} else {
		const lastMsg = history[history.length - 1];
		const ts = (lastMsg as SeqHistoryMessage).created_at;
		if (ts) appendLastMessageTimestamp(ts);
	}
	postHistoryLoadActions(key, searchContext, msgEls, thinkingText, skipAutoScroll === true);
}
