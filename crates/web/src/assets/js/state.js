// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// state module lives inside the bundle but is exposed on
// window.__moltis_state from app.tsx / onboarding-app.tsx.
//
// This shim re-exports everything the e2e tests need. All mutable
// values use `export let` with a requestAnimationFrame sync loop so
// reads always return the current value from the bundled state.

function state() {
	return window.__moltis_state || {};
}

// Default export — proxy to the bundled state namespace, even if imported early.
export default new Proxy({}, {
	get(_target, prop) {
		return state()[prop];
	},
});

// ── Live-synced state (export let + rAF) ────────────────────
// ES module `export let` creates live bindings that update when
// reassigned. We sync all mutable state on each animation frame.
export let connected = state().connected;
export let ws = state().ws;
export let pending = state().pending;
export let reqId = state().reqId;
export let activeSessionKey = state().activeSessionKey;
export let sessions = state().sessions;
export let models = state().models;
export let chatSeq = state().chatSeq;
export let chatInput = state().chatInput;
export let chatSendBtn = state().chatSendBtn;
export let chatMsgBox = state().chatMsgBox;
export let sessionTokens = state().sessionTokens;
export let sessionCurrentInputTokens = state().sessionCurrentInputTokens;
export let sessionCurrentContextTokens = state().sessionCurrentContextTokens;
export let sessionContextWindow = state().sessionContextWindow;
export let sessionToolsEnabled = state().sessionToolsEnabled;
export let sessionExecMode = state().sessionExecMode;
export let sessionExecPromptSymbol = state().sessionExecPromptSymbol;
export let commandModeEnabled = state().commandModeEnabled;
export let streamEl = state().streamEl;
export let streamText = state().streamText;
export let voicePending = state().voicePending;
export let sandboxInfo = state().sandboxInfo;
export let sessionSandboxEnabled = state().sessionSandboxEnabled;
export let sessionSandboxForced = state().sessionSandboxForced;
export let cachedChannels = state().cachedChannels;
export let selectedModelId = state().selectedModelId;
export let nodeCombo = state().nodeCombo;
export let nodeComboBtn = state().nodeComboBtn;
export let nodeComboLabel = state().nodeComboLabel;
export let nodeDropdown = state().nodeDropdown;
export let nodeDropdownList = state().nodeDropdownList;

// Sync all mutable state from the bundled namespace on each frame.
function _sync() {
	const S = state();
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
	sessionSandboxEnabled = S.sessionSandboxEnabled;
	sessionSandboxForced = S.sessionSandboxForced;
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
export function setConnected(v) { state().setConnected?.(v); connected = v; }
export function setWs(v) { state().setWs?.(v); ws = v; }
export function setReqId(v) { state().setReqId?.(v); reqId = v; }
export function setSubscribed(v) { state().setSubscribed?.(v); }
export function setModels(v) { state().setModels?.(v); models = v; }
export function setSessions(v) { state().setSessions?.(v); sessions = v; }
export function setActiveSessionKey(v) { state().setActiveSessionKey?.(v); activeSessionKey = v; }
export function setChatSeq(v) { state().setChatSeq?.(v); chatSeq = v; }
export function setChatInput(v) { state().setChatInput?.(v); chatInput = v; }
export function setChatSendBtn(v) { state().setChatSendBtn?.(v); chatSendBtn = v; }
export function setChatMsgBox(v) { state().setChatMsgBox?.(v); chatMsgBox = v; }
export function setStreamEl(v) { state().setStreamEl?.(v); streamEl = v; }
export function setStreamText(v) { state().setStreamText?.(v); streamText = v; }
export function setVoicePending(v) { state().setVoicePending?.(v); voicePending = v; }
export function setSessionTokens(v) { state().setSessionTokens?.(v); sessionTokens = v; }
export function setSessionCurrentInputTokens(v) { state().setSessionCurrentInputTokens?.(v); sessionCurrentInputTokens = v; }
export function setSessionCurrentContextTokens(v) { state().setSessionCurrentContextTokens?.(v); sessionCurrentContextTokens = v; }
export function setSessionContextWindow(v) { state().setSessionContextWindow?.(v); sessionContextWindow = v; }
export function setSessionToolsEnabled(v) { state().setSessionToolsEnabled?.(v); sessionToolsEnabled = v; }
export function setSessionExecMode(v) { state().setSessionExecMode?.(v); sessionExecMode = v; }
export function setSessionExecPromptSymbol(v) { state().setSessionExecPromptSymbol?.(v); sessionExecPromptSymbol = v; }
export function setCommandModeEnabled(v) { state().setCommandModeEnabled?.(v); commandModeEnabled = v; }
export function setSelectedModelId(v) { state().setSelectedModelId?.(v); selectedModelId = v; }
export function setSandboxInfo(v) { state().setSandboxInfo?.(v); sandboxInfo = v; }
export function setSessionSandboxEnabled(v) { state().setSessionSandboxEnabled?.(v); sessionSandboxEnabled = v; }
export function setSessionSandboxForced(v) { state().setSessionSandboxForced?.(v); sessionSandboxForced = v; }
export function setCachedChannels(v) { state().setCachedChannels?.(v); cachedChannels = v; }
export function setLastHistoryIndex(v) { state().setLastHistoryIndex?.(v); }
export function setSessionSwitchInProgress(v) { state().setSessionSwitchInProgress?.(v); }
export function setChatBatchLoading(v) { state().setChatBatchLoading?.(v); }
export function setHostExecIsRoot(v) { state().setHostExecIsRoot?.(v); }
export function setLogsEventHandler(v) { state().setLogsEventHandler?.(v); }
export function setNetworkAuditEventHandler(v) { state().setNetworkAuditEventHandler?.(v); }
export function setUnseenErrors(v) { state().setUnseenErrors?.(v); }
export function setUnseenWarns(v) { state().setUnseenWarns?.(v); }
export function setReconnectDelay(v) { state().setReconnectDelay?.(v); }
export function setNodeCombo(v) { state().setNodeCombo?.(v); nodeCombo = v; }
export function setNodeComboBtn(v) { state().setNodeComboBtn?.(v); nodeComboBtn = v; }
export function setNodeComboLabel(v) { state().setNodeComboLabel?.(v); nodeComboLabel = v; }
export function setNodeDropdown(v) { state().setNodeDropdown?.(v); nodeDropdown = v; }
export function setNodeDropdownList(v) { state().setNodeDropdownList?.(v); nodeDropdownList = v; }
export function setAutoScrollMode(v) { state().setAutoScrollMode?.(v); }

// DOM shorthand
export function $(id) { return state().$?.(id) ?? document.getElementById(id); }
