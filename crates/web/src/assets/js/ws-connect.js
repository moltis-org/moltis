// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// ws-connect module lives inside the bundle but is exposed on
// window.__moltis_modules["ws-connect"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["ws-connect"] || {};

export default M;

export const connectWs = (...args) => M.connectWs?.(...args);
