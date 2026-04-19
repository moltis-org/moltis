// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// chat-ui module lives inside the bundle but is exposed on
// window.__moltis_modules["chat-ui"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["chat-ui"] || {};

export default M;

export const chatAddMsg = (...args) => M.chatAddMsg?.(...args);
export const chatAddMsgWithImages = (...args) => M.chatAddMsgWithImages?.(...args);
export const updateTokenBar = (...args) => M.updateTokenBar?.(...args);
export const renderApprovalCard = (...args) => M.renderApprovalCard?.(...args);
export const updateCommandInputUI = (...args) => M.updateCommandInputUI?.(...args);
