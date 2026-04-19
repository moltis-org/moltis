// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// events module lives inside the bundle but is exposed on
// window.__moltis_modules["events"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["events"] || {};

export default M;

export const eventListeners = M.eventListeners;
export const onEvent = (...args) => M.onEvent?.(...args);
