// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// code-highlight module lives inside the bundle but is exposed on
// window.__moltis_modules["code-highlight"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["code-highlight"] || {};

export default M;

export const initHighlighter = (...args) => M.initHighlighter?.(...args);
export const highlightCodeBlocks = (...args) => M.highlightCodeBlocks?.(...args);
