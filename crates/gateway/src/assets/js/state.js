// ── Shared mutable state ────────────────────────────────────
import * as sig from "./signals.js";

export var ws = null;
export var reqId = 0;
export var connected = false;
export var reconnectDelay = 1000;
export var pending = {};
export var models = [];
export var activeSessionKey = localStorage.getItem("moltis-session") || "main";
export var activeProjectId = localStorage.getItem("moltis-project") || "";
export var sessions = [];
export var projects = [];

// Chat-page specific state (persists across page transitions)
export var streamEl = null;
export var streamText = "";
export var lastToolOutput = "";
export var voicePending = false;
export var chatHistory = JSON.parse(localStorage.getItem("moltis-chat-history") || "[]");
export var chatHistoryIdx = -1;
export var chatHistoryDraft = "";
// Client-side sequence counter for message ordering diagnostics.
// Resumed from the highest user seq in history on session switch.
export var chatSeq = 0;

// Session token usage tracking (cumulative for the current session)
export var sessionTokens = { input: 0, output: 0 };

// Model selector elements — created dynamically inside the chat page
export var modelCombo = null;
export var modelComboBtn = null;
export var modelComboLabel = null;
export var modelDropdown = null;
export var modelSearchInput = null;
export var modelDropdownList = null;
export var selectedModelId = localStorage.getItem("moltis-model") || "";
export var modelIdx = -1;

// Session project combo (in chat header)
export var projectCombo = null;
export var projectComboBtn = null;
export var projectComboLabel = null;
export var projectDropdown = null;
export var projectDropdownList = null;

// Sandbox toggle
export var sandboxToggleBtn = null;
export var sandboxLabel = null;
export var sessionSandboxEnabled = true;
export var sessionSandboxImage = null;
export var sandboxImageBtn = null;
export var sandboxImageDropdown = null;
export var sandboxImageLabel = null;

// Chat page DOM refs
export var chatMsgBox = null;
export var chatInput = null;
export var chatSendBtn = null;
export var chatBatchLoading = false;
export var sessionSwitchInProgress = false;
// Highest message index loaded from session history; used to deduplicate
// real-time events that duplicate already-rendered history entries.
export var lastHistoryIndex = -1;
export var sessionContextWindow = 0;
export var sessionToolsEnabled = true;

// Provider/channel page refresh callbacks
export var refreshProvidersPage = null;
export var refreshChannelsPage = null;
export var channelEventUnsub = null;

// Prefetched channel data
export var cachedChannels = null;
export function setCachedChannels(v) {
	cachedChannels = v;
	sig.cachedChannels.value = v;
}

// Sandbox
export var sandboxInfo = null;

// Logs
export var logsEventHandler = null;
export var unseenErrors = 0;
export var unseenWarns = 0;

// Project filter
export var projectFilterId = localStorage.getItem("moltis-project-filter") || "";

// DOM shorthand
export function $(id) {
	return document.getElementById(id);
}

// ── Setters ──────────────────────────────────────────────────
export function setWs(v) {
	ws = v;
}
export function setReqId(v) {
	reqId = v;
}
export function setConnected(v) {
	connected = v;
	sig.connected.value = v;
}
export function setReconnectDelay(v) {
	reconnectDelay = v;
}
export function setModels(v) {
	models = v;
	sig.models.value = v;
}
export function setActiveSessionKey(v) {
	activeSessionKey = v;
	sig.activeSessionKey.value = v;
}
export function setActiveProjectId(v) {
	activeProjectId = v;
}
export function setSessions(v) {
	sessions = v;
	sig.sessions.value = v;
}
export function setProjects(v) {
	projects = v;
	sig.projects.value = v;
}
export function setStreamEl(v) {
	streamEl = v;
}
export function setStreamText(v) {
	streamText = v;
}
export function setLastToolOutput(v) {
	lastToolOutput = v;
}
export function setVoicePending(v) {
	voicePending = v;
}
export function setChatHistory(v) {
	chatHistory = v;
}
export function setChatHistoryIdx(v) {
	chatHistoryIdx = v;
}
export function setChatHistoryDraft(v) {
	chatHistoryDraft = v;
}
export function setChatSeq(v) {
	chatSeq = v;
}
export function setSessionTokens(v) {
	sessionTokens = v;
}
export function setModelCombo(v) {
	modelCombo = v;
}
export function setModelComboBtn(v) {
	modelComboBtn = v;
}
export function setModelComboLabel(v) {
	modelComboLabel = v;
}
export function setModelDropdown(v) {
	modelDropdown = v;
}
export function setModelSearchInput(v) {
	modelSearchInput = v;
}
export function setModelDropdownList(v) {
	modelDropdownList = v;
}
export function setSelectedModelId(v) {
	selectedModelId = v;
	sig.selectedModelId.value = v;
}
export function setModelIdx(v) {
	modelIdx = v;
}
export function setProjectCombo(v) {
	projectCombo = v;
}
export function setProjectComboBtn(v) {
	projectComboBtn = v;
}
export function setProjectComboLabel(v) {
	projectComboLabel = v;
}
export function setProjectDropdown(v) {
	projectDropdown = v;
}
export function setProjectDropdownList(v) {
	projectDropdownList = v;
}
export function setSandboxToggleBtn(v) {
	sandboxToggleBtn = v;
}
export function setSandboxLabel(v) {
	sandboxLabel = v;
}
export function setSessionSandboxEnabled(v) {
	sessionSandboxEnabled = v;
}
export function setSessionSandboxImage(v) {
	sessionSandboxImage = v;
}
export function setSandboxImageBtn(v) {
	sandboxImageBtn = v;
}
export function setSandboxImageDropdown(v) {
	sandboxImageDropdown = v;
}
export function setSandboxImageLabel(v) {
	sandboxImageLabel = v;
}
export function setChatMsgBox(v) {
	chatMsgBox = v;
}
export function setChatInput(v) {
	chatInput = v;
}
export function setChatSendBtn(v) {
	chatSendBtn = v;
}
export function setChatBatchLoading(v) {
	chatBatchLoading = v;
}
export function setSessionSwitchInProgress(v) {
	sessionSwitchInProgress = v;
}
export function setLastHistoryIndex(v) {
	lastHistoryIndex = v;
}
export function setSessionContextWindow(v) {
	sessionContextWindow = v;
}
export function setSessionToolsEnabled(v) {
	sessionToolsEnabled = v;
}
export function setRefreshProvidersPage(v) {
	refreshProvidersPage = v;
}
export function setRefreshChannelsPage(v) {
	refreshChannelsPage = v;
}
export function setChannelEventUnsub(v) {
	channelEventUnsub = v;
}
export function setLogsEventHandler(v) {
	logsEventHandler = v;
}
export function setUnseenErrors(v) {
	unseenErrors = v;
	sig.unseenErrors.value = v;
}
export function setUnseenWarns(v) {
	unseenWarns = v;
	sig.unseenWarns.value = v;
}
export function setProjectFilterId(v) {
	projectFilterId = v;
}
export function setSandboxInfo(v) {
	sandboxInfo = v;
	sig.sandboxInfo.value = v;
}
