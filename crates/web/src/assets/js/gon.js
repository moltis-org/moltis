// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// gon module lives inside the bundle but is exposed on
// window.__moltis_modules["gon"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["gon"] || {};

export default M;

export const get = (...args) => M.get?.(...args);
export const set = (...args) => M.set?.(...args);
export const onChange = (...args) => M.onChange?.(...args);
export const refresh = (...args) => M.refresh?.(...args);
