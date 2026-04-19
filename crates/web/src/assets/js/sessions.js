// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// sessions module lives inside the bundle but is exposed on
// window.__moltis_modules["sessions"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["sessions"] || {};

export default M;

export const isArchivableSession = (...args) => M.isArchivableSession?.(...args);
export const fetchSessions = (...args) => M.fetchSessions?.(...args);
export const switchSession = (...args) => M.switchSession?.(...args);
export const renderSessionList = (...args) => M.renderSessionList?.(...args);
export const setSessionReplying = (...args) => M.setSessionReplying?.(...args);
