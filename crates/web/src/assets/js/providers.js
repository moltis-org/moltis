// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// providers module lives inside the bundle but is exposed on
// window.__moltis_modules["providers"] from main.tsx.
//
// This shim re-exports everything the e2e tests need.

const M = window.__moltis_modules?.["providers"] || {};

export default M;

export const openProviderModal = (...args) => M.openProviderModal?.(...args);
export const closeProviderModal = (...args) => M.closeProviderModal?.(...args);
export const getProviderModal = (...args) => M.getProviderModal?.(...args);
export const showModelDownloadProgress = (...args) => M.showModelDownloadProgress?.(...args);
export const showLocalModelFlow = (...args) => M.showLocalModelFlow?.(...args);
export const showApiKeyForm = (...args) => M.showApiKeyForm?.(...args);
export const showOAuthFlow = (...args) => M.showOAuthFlow?.(...args);
export const showCustomProviderForm = (...args) => M.showCustomProviderForm?.(...args);
export const openModelSelectorForProvider = (...args) => M.openModelSelectorForProvider?.(...args);
