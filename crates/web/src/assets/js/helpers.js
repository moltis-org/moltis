// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// helpers module lives inside the bundle but is exposed on
// window.__moltis_modules["helpers"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["helpers"] || {};

export default M;

export const localizeStructuredError = (...args) => M.localizeStructuredError?.(...args);
export const formatAssistantTokenUsage = (...args) => M.formatAssistantTokenUsage?.(...args);
export const formatTokens = (...args) => M.formatTokens?.(...args);
export const formatBytes = (...args) => M.formatBytes?.(...args);
export const sendRpc = (...args) => M.sendRpc?.(...args);
export const renderMarkdown = (...args) => M.renderMarkdown?.(...args);
export const esc = (...args) => M.esc?.(...args);
export const toolCallSummary = (...args) => M.toolCallSummary?.(...args);
export const formatAudioDuration = (...args) => M.formatAudioDuration?.(...args);
