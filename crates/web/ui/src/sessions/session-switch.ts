// ── Session switching: switch, restore, refresh ─────────────────

import { chatAddMsg, removeThinking, updateCommandInputUI, updateTokenBar } from "../chat-ui";
import { sendRpc } from "../helpers";
import { restoreNodeSelection } from "../nodes-selector";
import { updateSessionProjectSelect } from "../project-combo";
import { restoreReasoningFromModelId } from "../reasoning-toggle";
import { sessionPath } from "../router";
import { updateSandboxImageUI, updateSandboxUI } from "../sandbox";
import * as S from "../state";
import { modelStore } from "../stores/model-store";
import { projectStore } from "../stores/project-store";
import {
	clearSessionHistory,
	getHistoryRevision,
	getSessionHistory,
	replaceSessionHistory,
} from "../stores/session-history-cache";
import { insertSessionInOrder, Session, sessionStore } from "../stores/session-store";
import type { HistoryMessage, RpcResponse, SessionMeta } from "../types";

import {
	clearHistoryPaginationState,
	fetchSessionHistoryViaHttp,
	getHistoryPaginationState,
	type HistoryPayload,
	isHistoryCacheComplete,
	SESSION_HISTORY_PAGE_LIMIT,
	setHistoryPaginationState,
	shouldApplyServerHistory,
} from "./session-history";

import {
	hideSessionLoadIndicator,
	postHistoryLoadActions,
	renderHistory,
	type SearchContext,
	showSessionLoadIndicator,
	updateChatSessionHeader,
} from "./session-render";

/** Focus the chat input unless the user is actively editing a text field
 *  (e.g. rename input, search field). Buttons and other non-text elements
 *  are fine to steal focus from. */
function focusChatInputIfIdle(): void {
	const el = document.activeElement;
	if (el && el !== document.body && el !== S.chatInput) {
		const tag = el.tagName;
		if (tag === "INPUT" || tag === "TEXTAREA" || (el as HTMLElement).isContentEditable) return;
	}
	S.chatInput?.focus();
}

// ── Types ────────────────────────────────────────────────────

interface SwitchPayload {
	entry?: SessionMeta;
	history?: HistoryMessage[];
	historyCacheHit?: boolean;
	historyTruncated?: boolean;
	historyDroppedCount?: number;
	historyOmitted?: boolean;
	replying?: boolean;
	thinkingText?: string;
	voicePending?: boolean;
	hasMore?: boolean;
	nextCursor?: number;
	totalMessages?: number;
}

/** Parameters for the sessions.switch RPC call. */
interface SwitchRpcParams {
	key: string;
	project_id?: string;
	include_history?: boolean;
}

/** Sandbox runtime information from gon/server state. */
interface SandboxInfoPayload {
	backend?: string;
}

// ── Module state ─────────────────────────────────────────────

let switchRequestSeq = 0;
const latestSwitchRequestBySession = new Map<string, number>();

// ── MCP toggle restore ──────────────────────────────────────
function restoreMcpToggle(mcpEnabled: boolean): void {
	const mcpBtn = S.$("mcpToggleBtn");
	const mcpLabel = S.$("mcpToggleLabel");
	if (mcpBtn) {
		mcpBtn.style.color = mcpEnabled ? "var(--ok)" : "var(--muted)";
		mcpBtn.style.borderColor = mcpEnabled ? "var(--ok)" : "var(--border)";
	}
	if (mcpLabel) mcpLabel.textContent = mcpEnabled ? "MCP" : "MCP off";
}

// ── Restore session state ───────────────────────────────────

export function restoreSessionState(entry: SessionMeta, projectId?: string): void {
	const effectiveProjectId = entry.projectId || projectId || "";
	projectStore.setActiveProjectId(effectiveProjectId);
	S.setActiveProjectId(effectiveProjectId);
	localStorage.setItem("moltis-project", effectiveProjectId);
	updateSessionProjectSelect(effectiveProjectId);
	if (entry.model) {
		const baseModelId = restoreReasoningFromModelId(entry.model);
		modelStore.select(baseModelId);
		S.setSelectedModelId(baseModelId);
		localStorage.setItem("moltis-model", baseModelId);
		const found = modelStore.getById(baseModelId);
		if (S.modelComboLabel) S.modelComboLabel.textContent = found ? found.displayName || found.id : baseModelId;
	}
	updateSandboxUI(entry.sandbox_enabled !== false);
	updateSandboxImageUI(entry.sandbox_image || null);
	const sandboxRuntimeAvailable = ((S.sandboxInfo as SandboxInfoPayload | null)?.backend || "none") !== "none";
	const effectiveSandboxRoute = entry.sandbox_enabled !== false && sandboxRuntimeAvailable;
	S.setSessionExecMode(effectiveSandboxRoute ? "sandbox" : "host");
	S.setSessionExecPromptSymbol(effectiveSandboxRoute || S.hostExecIsRoot ? "#" : "$");
	updateCommandInputUI();
	restoreMcpToggle(!entry.mcpDisabled);
	restoreNodeSelection(entry.node_id || null);
	updateChatSessionHeader();
}

// ── Switch request tracking ─────────────────────────────────

export function startSwitchRequest(key: string): number {
	switchRequestSeq += 1;
	latestSwitchRequestBySession.set(key, switchRequestSeq);
	return switchRequestSeq;
}

function isLatestSwitchRequest(key: string, requestId: number): boolean {
	return latestSwitchRequestBySession.get(key) === requestId;
}

export function startSessionRefresh(key: string, blockRealtimeEvents: boolean): void {
	sessionStore.refreshInProgressKey.value = key;
	sessionStore.switchInProgress.value = !!blockRealtimeEvents;
	S.setSessionSwitchInProgress(!!blockRealtimeEvents);
}

function finishSessionRefresh(key: string): void {
	if (sessionStore.refreshInProgressKey.value !== key) return;
	sessionStore.refreshInProgressKey.value = "";
	sessionStore.switchInProgress.value = false;
	S.setSessionSwitchInProgress(false);
}

function resetSwitchViewState(): void {
	hideSessionLoadIndicator();
	if (S.chatMsgBox) S.chatMsgBox.textContent = "";
	const tray = document.getElementById("queuedMessages");
	if (tray) {
		while (tray.firstChild) tray.removeChild(tray.firstChild);
		tray.classList.add("hidden");
	}
	S.setStreamEl(null);
	S.setStreamText("");
	S.setLastToolOutput("");
	S.setVoicePending(false);
	S.setLastHistoryIndex(-1);
	S.setSessionTokens({ input: 0, output: 0 });
	S.setSessionCurrentInputTokens(0);
	S.setSessionContextWindow(0);
	updateTokenBar();
}

function ensureSessionInClientStore(key: string, entry: SessionMeta, projectId?: string): unknown {
	const existing = sessionStore.getByKey(key);
	if (existing) return existing;

	const created: SessionMeta = { ...entry, key };
	if (projectId && !created.projectId) created.projectId = projectId;
	const createdSession = sessionStore.upsert(created);

	const inLegacy = (S.sessions as SessionMeta[]).some((s) => s.key === key);
	if (!inLegacy) {
		S.setSessions(insertSessionInOrder(S.sessions as Session[], new Session(created)));
	}
	return createdSession;
}

function applyReplyingStateFromSwitchPayload(key: string, payload: SwitchPayload): void {
	const replying = payload.replying === true;
	// Lazy-load setSessionReplying from parent to avoid circular imports
	const session = sessionStore.getByKey(key);
	if (session) session.replying.value = replying;
	const entry = (S.sessions as SessionMeta[]).find((s) => s.key === key);
	if (entry) entry._replying = replying;

	const voiceSession = sessionStore.getByKey(key);
	if (replying && payload.voicePending) {
		S.setVoicePending(true);
		if (voiceSession) voiceSession.voicePending.value = true;
	} else {
		S.setVoicePending(false);
		if (voiceSession) voiceSession.voicePending.value = false;
	}
	if (!replying && key === sessionStore.activeSessionKey.value) {
		removeThinking();
	}
}

/** Clear history for the currently active session and reset local UI state. */
export function clearActiveSession(): Promise<RpcResponse> {
	const prevHistoryIdx = S.lastHistoryIndex;
	const prevSeq = S.chatSeq;
	S.setLastHistoryIndex(-1);
	S.setChatSeq(0);
	return sendRpc("chat.clear", {}).then((res) => {
		if (res?.ok) {
			if (S.chatMsgBox) S.chatMsgBox.textContent = "";
			S.setSessionTokens({ input: 0, output: 0 });
			S.setSessionCurrentInputTokens(0);
			updateTokenBar();
			const activeKey = sessionStore.activeSessionKey.value || S.activeSessionKey;
			// Inline markSessionLocallyCleared to avoid circular import
			void import("../sessions").then(({ markSessionLocallyCleared }) => markSessionLocallyCleared(activeKey));
			clearSessionHistory(activeKey);
			clearHistoryPaginationState(activeKey);
			return res;
		}
		S.setLastHistoryIndex(prevHistoryIdx);
		S.setChatSeq(prevSeq);
		chatAddMsg("error", res?.error?.message || "Clear failed");
		return res;
	});
}

// ── Main switch session function ────────────────────────────

export function switchSession(key: string, searchContext?: SearchContext | null, projectId?: string): void {
	sessionStore.setActive(key);
	S.setActiveSessionKey(key);
	localStorage.setItem("moltis-session", key);
	history.replaceState(null, "", sessionPath(key));
	resetSwitchViewState();
	const cachedEntry = sessionStore.getByKey(key);
	if (cachedEntry) {
		restoreSessionState(cachedEntry.toMeta(), projectId);
	}

	const switchReqId = startSwitchRequest(key);
	const switchParams: SwitchRpcParams = { key, include_history: false };
	if (projectId) switchParams.project_id = projectId;
	const cachedHistory = getSessionHistory(key);
	const hasCache = Array.isArray(cachedHistory);
	const cacheRevisionAtRequest = getHistoryRevision(key);
	const cacheComplete = hasCache && isHistoryCacheComplete(key);
	const cachedHistoryCount = cacheComplete
		? Number.isInteger(cachedEntry?.messageCount)
			? (cachedEntry?.messageCount as number)
			: cachedHistory?.length
		: null;
	startSessionRefresh(key, !hasCache);
	if (hasCache) {
		renderHistory(key, cachedHistory!, searchContext || null, null, cachedHistoryCount, false);
	} else {
		showSessionLoadIndicator();
	}

	sendRpc("sessions.switch", switchParams)
		.then(async (res) => {
			if (!isLatestSwitchRequest(key, switchReqId)) return;
			let stillActive = sessionStore.activeSessionKey.value === key;
			if (!(res?.ok && res.payload)) {
				if (stillActive && !hasCache) {
					hideSessionLoadIndicator();
					chatAddMsg("error", res?.error?.message || "Failed to load session");
				}
				finishSessionRefresh(key);
				if (stillActive) focusChatInputIfIdle();
				return;
			}

			const switchPayload = res.payload as SwitchPayload;
			const entry = switchPayload.entry || ({} as SessionMeta);
			ensureSessionInClientStore(key, entry, projectId);
			const pagingBefore = getHistoryPaginationState(key);
			const pagingBeforeHasMore = pagingBefore?.hasMore === true;
			const pagingBeforeCursor = Number.isInteger(pagingBefore?.nextCursor) ? pagingBefore?.nextCursor : null;
			let historyPayload: HistoryPayload = {
				historyCacheHit: switchPayload.historyCacheHit === true,
				history: Array.isArray(switchPayload.history) ? switchPayload.history : [],
				historyTruncated: switchPayload.historyTruncated === true,
				historyDroppedCount: Number(switchPayload.historyDroppedCount) || 0,
			};
			if (switchPayload.historyOmitted === true) {
				try {
					historyPayload = await fetchSessionHistoryViaHttp(key, {
						cachedMessageCount: cachedHistoryCount ?? undefined,
						limit: SESSION_HISTORY_PAGE_LIMIT,
					});
				} catch (error) {
					if (!isLatestSwitchRequest(key, switchReqId)) return;
					stillActive = sessionStore.activeSessionKey.value === key;
					if (stillActive && !hasCache) {
						hideSessionLoadIndicator();
						chatAddMsg("error", (error as Error)?.message || "Failed to load session history");
					}
					finishSessionRefresh(key);
					if (stillActive) focusChatInputIfIdle();
					return;
				}
				if (!isLatestSwitchRequest(key, switchReqId)) return;
				stillActive = sessionStore.activeSessionKey.value === key;
			}
			setHistoryPaginationState(key, historyPayload);
			const pagingAfter = getHistoryPaginationState(key);
			const pagingAfterHasMore = pagingAfter?.hasMore === true;
			const pagingAfterCursor = Number.isInteger(pagingAfter?.nextCursor) ? pagingAfter?.nextCursor : null;
			const paginationChanged = pagingBeforeHasMore !== pagingAfterHasMore || pagingBeforeCursor !== pagingAfterCursor;

			const cacheHit = historyPayload.historyCacheHit === true;
			const serverHistory = Array.isArray(historyPayload.history) ? historyPayload.history : [];
			let appliedServerHistory = false;
			if (!cacheHit && shouldApplyServerHistory(key, serverHistory, cacheRevisionAtRequest)) {
				replaceSessionHistory(key, serverHistory);
				appliedServerHistory = true;
			}
			const resolvedHistory = getSessionHistory(key) || serverHistory;
			if (stillActive) {
				restoreSessionState(entry, projectId);
				applyReplyingStateFromSwitchPayload(key, switchPayload);
				const thinkingText = switchPayload.replying ? switchPayload.thinkingText || null : null;
				const totalCountHint = Number.isInteger(entry.messageCount)
					? entry.messageCount!
					: Number(historyPayload.totalMessages) || resolvedHistory.length;
				const shouldRerender = !hasCache || Boolean(searchContext?.query) || appliedServerHistory || paginationChanged;
				if (shouldRerender) {
					renderHistory(key, resolvedHistory, searchContext || null, thinkingText, totalCountHint, false);
				} else {
					postHistoryLoadActions(key, searchContext || null, [], thinkingText, false);
				}
				if (appliedServerHistory && historyPayload.historyTruncated === true) {
					const dropped = Number(historyPayload.historyDroppedCount) || 0;
					chatAddMsg(
						"system",
						`Loaded the most recent messages for performance (${dropped} older message${dropped === 1 ? "" : "s"} omitted).`,
					);
				}
				if (appliedServerHistory && historyPayload.hasMore === true) {
					const total = Number(historyPayload.totalMessages) || resolvedHistory.length;
					chatAddMsg(
						"system",
						`Loaded recent history (${resolvedHistory.length} of ${total} messages) for faster loading.`,
					);
				}
				focusChatInputIfIdle();
			}
			finishSessionRefresh(key);
		})
		.catch(() => {
			if (!isLatestSwitchRequest(key, switchReqId)) return;
			const stillActive = sessionStore.activeSessionKey.value === key;
			if (stillActive && !hasCache) {
				hideSessionLoadIndicator();
				chatAddMsg("error", "Failed to load session");
			}
			finishSessionRefresh(key);
			if (stillActive) focusChatInputIfIdle();
		});
}
