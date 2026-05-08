// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// page-channels module lives inside the bundle but is exposed on
// window.__moltis_modules["page-channels"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["page-channels"] || {};

export default M;

export const prefetchChannels = (...args) => M.prefetchChannels?.(...args);
export const initChannels = (...args) => M.initChannels?.(...args);
export const teardownChannels = (...args) => M.teardownChannels?.(...args);
