// E2E test compatibility shim.
//
// With Vite bundling, the real events module lives inside the bundle
// and is exposed on window.__moltis_modules["events"] from app.tsx.
//
// This shim re-exports everything the e2e tests need, proxying to the
// bundled module so that event subscriptions share the same bus.

const M = window.__moltis_modules?.["events"] || {};

export const eventListeners = M.eventListeners || {};
export const onEvent = (...args) => M.onEvent?.(...args);
