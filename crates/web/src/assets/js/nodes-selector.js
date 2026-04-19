// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// nodes-selector module lives inside the bundle but is exposed on
// window.__moltis_modules["nodes-selector"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["nodes-selector"] || {};

export default M;

export const fetchNodes = (...args) => M.fetchNodes?.(...args);
export const bindNodeComboEvents = (...args) => M.bindNodeComboEvents?.(...args);
