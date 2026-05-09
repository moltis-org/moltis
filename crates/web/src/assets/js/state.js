// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// state module lives inside the bundle but is exposed on
// window.__moltis_state from app.tsx / onboarding-app.tsx.
//
// This shim re-exports everything the e2e tests need. All mutable
// values use `export let` with a requestAnimationFrame sync loop so
// reads always return the current value from the bundled state.

const S = window.__moltis_state || {};

// Default export — direct reference to the bundled state namespace
export default S;

// ── Live-synced state (export let + rAF) ────────────────────
// ES module `export let` creates live bindings that update when
// reassigned. We sync all mutable state on each animation frame.
export let connected = S.connected;
export let ws = S.ws;
export let pending = S.pending;
export let reqId = S.reqId;
export let activeSessionKey = S.activeSessionKey;
export let sessions = S.sessions;
export let models = S.models;
export let chatSeq = S.chatSeq;
export let chatInput = S.chatInput;
export let chatSendBtn = S.chatSendBtn;
export let chatMsgBox = S.chatMsgBox;
export let sessionTokens = S.sessionTokens;
export let sessionCurrentInputTokens = S.sessionCurrentInputTokens;
export let sessionCurrentContextTokens = S.sessionCurrentContextTokens;
export let sessionContextWindow = S.sessionContextWindow;
export let sessionToolsEnabled = S.sessionToolsEnabled;
export let sessionExecMode = S.sessionExecMode;
export let sessionExecPromptSymbol = S.sessionExecPromptSymbol;
export let commandModeEnabled = S.commandModeEnabled;
export let streamEl = S.streamEl;
export let streamText = S.streamText;
export let voicePending = S.voicePending;
export let sandboxInfo = S.sandboxInfo;
export let cachedChannels = S.cachedChannels;
export let selectedModelId = S.selectedModelId;
export let nodeCombo = S.nodeCombo;
export let nodeComboBtn = S.nodeComboBtn;
export let nodeComboLabel = S.nodeComboLabel;
export let nodeDropdown = S.nodeDropdown;
export let nodeDropdownList = S.nodeDropdownList;

// Sync all mutable state from the bundled namespace on each frame.
function _sync() {
	connected = S.connected;
	ws = S.ws;
	pending = S.pending;
	reqId = S.reqId;
	activeSessionKey = S.activeSessionKey;
	sessions = S.sessions;
	models = S.models;
	chatSeq = S.chatSeq;
	chatInput = S.chatInput;
	chatSendBtn = S.chatSendBtn;
	chatMsgBox = S.chatMsgBox;
	sessionTokens = S.sessionTokens;
	sessionCurrentInputTokens = S.sessionCurrentInputTokens;
	sessionCurrentContextTokens = S.sessionCurrentContextTokens;
	sessionContextWindow = S.sessionContextWindow;
	sessionToolsEnabled = S.sessionToolsEnabled;
	sessionExecMode = S.sessionExecMode;
	sessionExecPromptSymbol = S.sessionExecPromptSymbol;
	commandModeEnabled = S.commandModeEnabled;
	streamEl = S.streamEl;
	streamText = S.streamText;
	voicePending = S.voicePending;
	sandboxInfo = S.sandboxInfo;
	cachedChannels = S.cachedChannels;
	selectedModelId = S.selectedModelId;
	nodeCombo = S.nodeCombo;
	nodeComboBtn = S.nodeComboBtn;
	nodeComboLabel = S.nodeComboLabel;
	nodeDropdown = S.nodeDropdown;
	nodeDropdownList = S.nodeDropdownList;
	requestAnimationFrame(_sync);
}
requestAnimationFrame(_sync);

// ── Setters (proxy to real state module) ────────────────────
export function setConnected(v) { S.setConnected?.(v); connected = v; }
export function setWs(v) { S.setWs?.(v); ws = v; }
export function setReqId(v) { S.setReqId?.(v); reqId = v; }
export function setSubscribed(v) { S.setSubscribed?.(v); }
export function setModels(v) { S.setModels?.(v); models = v; }
export function setSessions(v) { S.setSessions?.(v); sessions = v; }
export function setActiveSessionKey(v) { S.setActiveSessionKey?.(v); activeSessionKey = v; }
export function setChatSeq(v) { S.setChatSeq?.(v); chatSeq = v; }
export function setChatInput(v) { S.setChatInput?.(v); chatInput = v; }
export function setChatSendBtn(v) { S.setChatSendBtn?.(v); chatSendBtn = v; }
export function setChatMsgBox(v) { S.setChatMsgBox?.(v); chatMsgBox = v; }
export function setStreamEl(v) { S.setStreamEl?.(v); streamEl = v; }
export function setStreamText(v) { S.setStreamText?.(v); streamText = v; }
export function setVoicePending(v) { S.setVoicePending?.(v); voicePending = v; }
export function setSessionTokens(v) { S.setSessionTokens?.(v); sessionTokens = v; }
export function setSessionCurrentInputTokens(v) { S.setSessionCurrentInputTokens?.(v); sessionCurrentInputTokens = v; }
export function setSessionCurrentContextTokens(v) { S.setSessionCurrentContextTokens?.(v); sessionCurrentContextTokens = v; }
export function setSessionContextWindow(v) { S.setSessionContextWindow?.(v); sessionContextWindow = v; }
export function setSessionToolsEnabled(v) { S.setSessionToolsEnabled?.(v); sessionToolsEnabled = v; }
export function setSessionExecMode(v) { S.setSessionExecMode?.(v); sessionExecMode = v; }
export function setSessionExecPromptSymbol(v) { S.setSessionExecPromptSymbol?.(v); sessionExecPromptSymbol = v; }
export function setCommandModeEnabled(v) { S.setCommandModeEnabled?.(v); commandModeEnabled = v; }
export function setSelectedModelId(v) { S.setSelectedModelId?.(v); selectedModelId = v; }
export function setSandboxInfo(v) { S.setSandboxInfo?.(v); sandboxInfo = v; }
export function setCachedChannels(v) { S.setCachedChannels?.(v); cachedChannels = v; }
export function setLastHistoryIndex(v) { S.setLastHistoryIndex?.(v); }
export function setSessionSwitchInProgress(v) { S.setSessionSwitchInProgress?.(v); }
export function setChatBatchLoading(v) { S.setChatBatchLoading?.(v); }
export function setHostExecIsRoot(v) { S.setHostExecIsRoot?.(v); }
export function setLogsEventHandler(v) { S.setLogsEventHandler?.(v); }
export function setNetworkAuditEventHandler(v) { S.setNetworkAuditEventHandler?.(v); }
export function setUnseenErrors(v) { S.setUnseenErrors?.(v); }
export function setUnseenWarns(v) { S.setUnseenWarns?.(v); }
export function setReconnectDelay(v) { S.setReconnectDelay?.(v); }
export function setNodeCombo(v) { S.setNodeCombo?.(v); nodeCombo = v; }
export function setNodeComboBtn(v) { S.setNodeComboBtn?.(v); nodeComboBtn = v; }
export function setNodeComboLabel(v) { S.setNodeComboLabel?.(v); nodeComboLabel = v; }
export function setNodeDropdown(v) { S.setNodeDropdown?.(v); nodeDropdown = v; }
export function setNodeDropdownList(v) { S.setNodeDropdownList?.(v); nodeDropdownList = v; }
export function setAutoScrollMode(v) { S.setAutoScrollMode?.(v); }

// DOM shorthand
export function $(id) { return S.$?.(id) ?? document.getElementById(id); }
