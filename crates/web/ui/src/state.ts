// ── Shared mutable state ────────────────────────────────────

import * as sig from "./signals";
import type { RpcResponse, SessionTokens } from "./types";

export let ws: WebSocket | null = null;
export let reqId = 0;
export let connected = false;
export let subscribed = false;
export let reconnectDelay = 1000;
export const pending: Record<string, (value: RpcResponse) => void> = {};
export let models: unknown[] = [];
export let activeSessionKey: string = localStorage.getItem("moltis-session") || "main";
export let activeProjectId: string = localStorage.getItem("moltis-project") || "";
export let sessions: unknown[] = [];
export let projects: unknown[] = [];

// Chat-page specific state (persists across page transitions)
export let streamEl: HTMLElement | null = null;
export let streamText = "";
export let lastToolOutput = "";
export let voicePending = false;
export let chatHistory: string[] = JSON.parse(localStorage.getItem("moltis-chat-history") || "[]");
export let chatHistoryIdx = -1;
export let chatHistoryDraft = "";
// Client-side sequence counter for message ordering diagnostics.
// Resumed from the highest user seq in history on session switch.
export let chatSeq = 0;

// Session token usage tracking (cumulative for the current session)
export let sessionTokens: SessionTokens = { input: 0, output: 0 };
// Last observed prompt input tokens (context pressure for the next turn).
export let sessionCurrentInputTokens = 0;

// Model selector elements — created dynamically inside the chat page
export let modelCombo: HTMLElement | null = null;
export let modelComboBtn: HTMLElement | null = null;
export let modelComboLabel: HTMLElement | null = null;
export let modelDropdown: HTMLElement | null = null;
export let modelSearchInput: HTMLElement | null = null;
export let modelDropdownList: HTMLElement | null = null;
export let selectedModelId: string = localStorage.getItem("moltis-model") || "";
export let modelIdx = -1;

// Node selector elements — created dynamically inside the chat page
export let nodeCombo: HTMLElement | null = null;
export let nodeComboBtn: HTMLElement | null = null;
export let nodeComboLabel: HTMLElement | null = null;
export let nodeDropdown: HTMLElement | null = null;
export let nodeDropdownList: HTMLElement | null = null;

// Session project combo (in chat header)
export let projectCombo: HTMLElement | null = null;
export let projectComboBtn: HTMLElement | null = null;
export let projectComboLabel: HTMLElement | null = null;
export let projectDropdown: HTMLElement | null = null;
export let projectDropdownList: HTMLElement | null = null;

// Sandbox toggle
export let sandboxToggleBtn: HTMLButtonElement | null = null;
export let sandboxLabel: HTMLElement | null = null;
export let sessionSandboxEnabled = true;
export let sessionSandboxImage: string | null = null;
export let sandboxImageBtn: HTMLButtonElement | null = null;
export let sandboxImageDropdown: HTMLElement | null = null;
export let sandboxImageLabel: HTMLElement | null = null;

// Chat page DOM refs
export let chatMsgBox: HTMLElement | null = null;
export let chatInput: HTMLElement | null = null;
export let chatSendBtn: HTMLElement | null = null;
export let chatBatchLoading = false;
export let autoScrollMode: string = localStorage.getItem("moltis-auto-scroll") || "smart";
export let sessionSwitchInProgress = false;
// Highest message index loaded from session history; used to deduplicate
// real-time events that duplicate already-rendered history entries.
export let lastHistoryIndex = -1;
export let sessionContextWindow = 0;
export let sessionToolsEnabled = true;
export let sessionExecMode = "host";
export let sessionExecPromptSymbol = "$";
export let hostExecIsRoot = false;
export let commandModeEnabled = false;

// Provider/channel page refresh callbacks
export let refreshProvidersPage: (() => void) | null = null;
export let refreshChannelsPage: (() => void) | null = null;
export let channelEventUnsub: (() => void) | null = null;

// Prefetched channel data
export let cachedChannels: unknown | null = null;
export function setCachedChannels(v: unknown | null): void {
	cachedChannels = v;
	sig.cachedChannels.value = v;
}

// Sandbox
export let sandboxInfo: unknown | null = null;

// Logs
export let logsEventHandler: ((payload?: unknown) => void) | null = null;

// Network audit
export let networkAuditEventHandler: ((payload?: unknown) => void) | null = null;
export let unseenErrors = 0;
export let unseenWarns = 0;

// Project filter
export let projectFilterId: string = localStorage.getItem("moltis-project-filter") || "";

// DOM shorthand
export function $<T extends HTMLElement = HTMLElement>(id: string): T | null {
	return document.getElementById(id) as T | null;
}

// ── Setters ──────────────────────────────────────────────────
export function setWs(v: WebSocket | null): void {
	ws = v;
}
export function setReqId(v: number): void {
	reqId = v;
}
export function setConnected(v: boolean): void {
	connected = v;
	sig.connected.value = v;
}
export function setSubscribed(v: boolean): void {
	subscribed = v;
}
export function setReconnectDelay(v: number): void {
	reconnectDelay = v;
}
export function setModels(v: unknown[]): void {
	models = v;
	// Store signal is now owned by model-store.ts; don't overwrite here.
}
export function setActiveSessionKey(v: string): void {
	activeSessionKey = v;
	// Store signal is now owned by session-store.ts; don't overwrite here.
}
export function setActiveProjectId(v: string): void {
	activeProjectId = v;
}
export function setSessions(v: unknown[]): void {
	sessions = v;
	// Store signal is now owned by session-store.ts; don't overwrite here.
}
export function setProjects(v: unknown[]): void {
	projects = v;
	// Store signal is now owned by project-store.ts; don't overwrite here.
}
export function setStreamEl(v: HTMLElement | null): void {
	streamEl = v;
}
export function setStreamText(v: string): void {
	streamText = v;
}
export function setLastToolOutput(v: string): void {
	lastToolOutput = v;
}
export function setVoicePending(v: boolean): void {
	voicePending = v;
}
export function setChatHistory(v: string[]): void {
	chatHistory = v;
}
export function setChatHistoryIdx(v: number): void {
	chatHistoryIdx = v;
}
export function setChatHistoryDraft(v: string): void {
	chatHistoryDraft = v;
}
export function setChatSeq(v: number): void {
	chatSeq = v;
}
export function setSessionTokens(v: SessionTokens): void {
	sessionTokens = v;
}
export function setSessionCurrentInputTokens(v: number): void {
	sessionCurrentInputTokens = v;
}
export function setModelCombo(v: HTMLElement | null): void {
	modelCombo = v;
}
export function setModelComboBtn(v: HTMLElement | null): void {
	modelComboBtn = v;
}
export function setModelComboLabel(v: HTMLElement | null): void {
	modelComboLabel = v;
}
export function setModelDropdown(v: HTMLElement | null): void {
	modelDropdown = v;
}
export function setModelSearchInput(v: HTMLElement | null): void {
	modelSearchInput = v;
}
export function setModelDropdownList(v: HTMLElement | null): void {
	modelDropdownList = v;
}
export function setSelectedModelId(v: string): void {
	selectedModelId = v;
	// Store signal is now owned by model-store.ts; don't overwrite here.
}
export function setModelIdx(v: number): void {
	modelIdx = v;
}
export function setNodeCombo(v: HTMLElement | null): void {
	nodeCombo = v;
}
export function setNodeComboBtn(v: HTMLElement | null): void {
	nodeComboBtn = v;
}
export function setNodeComboLabel(v: HTMLElement | null): void {
	nodeComboLabel = v;
}
export function setNodeDropdown(v: HTMLElement | null): void {
	nodeDropdown = v;
}
export function setNodeDropdownList(v: HTMLElement | null): void {
	nodeDropdownList = v;
}
export function setProjectCombo(v: HTMLElement | null): void {
	projectCombo = v;
}
export function setProjectComboBtn(v: HTMLElement | null): void {
	projectComboBtn = v;
}
export function setProjectComboLabel(v: HTMLElement | null): void {
	projectComboLabel = v;
}
export function setProjectDropdown(v: HTMLElement | null): void {
	projectDropdown = v;
}
export function setProjectDropdownList(v: HTMLElement | null): void {
	projectDropdownList = v;
}
export function setSandboxToggleBtn(v: HTMLButtonElement | null): void {
	sandboxToggleBtn = v;
}
export function setSandboxLabel(v: HTMLElement | null): void {
	sandboxLabel = v;
}
export function setSessionSandboxEnabled(v: boolean): void {
	sessionSandboxEnabled = v;
}
export function setSessionSandboxImage(v: string | null): void {
	sessionSandboxImage = v;
}
export function setSandboxImageBtn(v: HTMLButtonElement | null): void {
	sandboxImageBtn = v;
}
export function setSandboxImageDropdown(v: HTMLElement | null): void {
	sandboxImageDropdown = v;
}
export function setSandboxImageLabel(v: HTMLElement | null): void {
	sandboxImageLabel = v;
}
export function setChatMsgBox(v: HTMLElement | null): void {
	chatMsgBox = v;
}
export function setChatInput(v: HTMLElement | null): void {
	chatInput = v;
}
export function setChatSendBtn(v: HTMLElement | null): void {
	chatSendBtn = v;
}
export function setChatBatchLoading(v: boolean): void {
	chatBatchLoading = v;
}
export function setAutoScrollMode(v: string): void {
	autoScrollMode = v;
	localStorage.setItem("moltis-auto-scroll", v);
}
export function setSessionSwitchInProgress(v: boolean): void {
	sessionSwitchInProgress = v;
}
export function setLastHistoryIndex(v: number): void {
	lastHistoryIndex = v;
}
export function setSessionContextWindow(v: number): void {
	sessionContextWindow = v;
}
export function setSessionToolsEnabled(v: boolean): void {
	sessionToolsEnabled = v;
}
export function setSessionExecMode(v: string): void {
	sessionExecMode = v;
}
export function setSessionExecPromptSymbol(v: string): void {
	sessionExecPromptSymbol = v;
}
export function setHostExecIsRoot(v: boolean): void {
	hostExecIsRoot = !!v;
}
export function setCommandModeEnabled(v: boolean): void {
	commandModeEnabled = !!v;
}
export function setRefreshProvidersPage(v: (() => void) | null): void {
	refreshProvidersPage = v;
}
export function setRefreshChannelsPage(v: (() => void) | null): void {
	refreshChannelsPage = v;
}
export function setChannelEventUnsub(v: (() => void) | null): void {
	channelEventUnsub = v;
}
export function setLogsEventHandler(v: ((payload?: unknown) => void) | null): void {
	logsEventHandler = v;
}
export function setNetworkAuditEventHandler(v: ((payload?: unknown) => void) | null): void {
	networkAuditEventHandler = v;
}
export function setUnseenErrors(v: number): void {
	unseenErrors = v;
	sig.unseenErrors.value = v;
}
export function setUnseenWarns(v: number): void {
	unseenWarns = v;
	sig.unseenWarns.value = v;
}
export function setProjectFilterId(v: string): void {
	projectFilterId = v;
}
export function setSandboxInfo(v: unknown | null): void {
	sandboxInfo = v;
	sig.sandboxInfo.value = v;
}
