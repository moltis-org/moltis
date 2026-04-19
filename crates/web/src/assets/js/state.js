// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// state module lives inside the bundle but is exposed on
// window.__moltis_state from main.tsx / onboarding-app.tsx.
//
// This shim re-exports everything the e2e tests need. The tests do
// `const state = await import("${prefix}js/state.js")` and then call
// setters like `state.setConnected(true)` and read `state.pending`.

const S = window.__moltis_state || {};

// Default export for `state.xyz` access pattern
export default S;

// ── Read-only state properties ──────────────────────────────
// These are captured at import time. For mutable values (connected, ws)
// the tests always call the setter first, so stale reads are fine.
export const connected = S.connected;
export const ws = S.ws;
export const pending = S.pending;
export const reqId = S.reqId;
export const activeSessionKey = S.activeSessionKey;
export const sessions = S.sessions;
export const models = S.models;
export const chatSeq = S.chatSeq;
export const chatInput = S.chatInput;
export const chatSendBtn = S.chatSendBtn;
export const chatMsgBox = S.chatMsgBox;
export const sessionTokens = S.sessionTokens;
export const sessionCurrentInputTokens = S.sessionCurrentInputTokens;
export const sessionContextWindow = S.sessionContextWindow;
export const sessionToolsEnabled = S.sessionToolsEnabled;
export const sessionExecMode = S.sessionExecMode;
export const sessionExecPromptSymbol = S.sessionExecPromptSymbol;
export const commandModeEnabled = S.commandModeEnabled;
export const streamEl = S.streamEl;
export const streamText = S.streamText;
export const voicePending = S.voicePending;
export const sandboxInfo = S.sandboxInfo;
export const cachedChannels = S.cachedChannels;
export const selectedModelId = S.selectedModelId;

// ── Setters (proxy to real state module) ────────────────────
export function setConnected(v) { S.setConnected?.(v); }
export function setWs(v) { S.setWs?.(v); }
export function setReqId(v) { S.setReqId?.(v); }
export function setSubscribed(v) { S.setSubscribed?.(v); }
export function setModels(v) { S.setModels?.(v); }
export function setSessions(v) { S.setSessions?.(v); }
export function setActiveSessionKey(v) { S.setActiveSessionKey?.(v); }
export function setChatSeq(v) { S.setChatSeq?.(v); }
export function setChatInput(v) { S.setChatInput?.(v); }
export function setChatSendBtn(v) { S.setChatSendBtn?.(v); }
export function setChatMsgBox(v) { S.setChatMsgBox?.(v); }
export function setStreamEl(v) { S.setStreamEl?.(v); }
export function setStreamText(v) { S.setStreamText?.(v); }
export function setVoicePending(v) { S.setVoicePending?.(v); }
export function setSessionTokens(v) { S.setSessionTokens?.(v); }
export function setSessionCurrentInputTokens(v) { S.setSessionCurrentInputTokens?.(v); }
export function setSessionContextWindow(v) { S.setSessionContextWindow?.(v); }
export function setSessionToolsEnabled(v) { S.setSessionToolsEnabled?.(v); }
export function setSessionExecMode(v) { S.setSessionExecMode?.(v); }
export function setSessionExecPromptSymbol(v) { S.setSessionExecPromptSymbol?.(v); }
export function setCommandModeEnabled(v) { S.setCommandModeEnabled?.(v); }
export function setSelectedModelId(v) { S.setSelectedModelId?.(v); }
export function setSandboxInfo(v) { S.setSandboxInfo?.(v); }
export function setCachedChannels(v) { S.setCachedChannels?.(v); }
export function setLastHistoryIndex(v) { S.setLastHistoryIndex?.(v); }
export function setSessionSwitchInProgress(v) { S.setSessionSwitchInProgress?.(v); }
export function setChatBatchLoading(v) { S.setChatBatchLoading?.(v); }
export function setHostExecIsRoot(v) { S.setHostExecIsRoot?.(v); }
export function setLogsEventHandler(v) { S.setLogsEventHandler?.(v); }
export function setNetworkAuditEventHandler(v) { S.setNetworkAuditEventHandler?.(v); }
export function setUnseenErrors(v) { S.setUnseenErrors?.(v); }
export function setUnseenWarns(v) { S.setUnseenWarns?.(v); }
export function setReconnectDelay(v) { S.setReconnectDelay?.(v); }

// DOM shorthand
export function $(id) { return S.$?.(id) ?? document.getElementById(id); }
