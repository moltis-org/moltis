// E2E test compatibility shim.
//
// With Vite bundling, individual modules are no longer served. The real
// sandbox module lives inside the bundle but is exposed on
// window.__moltis_modules["sandbox"] from app.tsx.
//
// This shim re-exports everything the e2e tests need.

function sandbox() {
	return window.__moltis_modules?.sandbox || {};
}

export default new Proxy({}, {
	get(_target, prop) {
		return sandbox()[prop];
	},
});

export const updateSandboxUI = (...args) => sandbox().updateSandboxUI?.(...args);
export const updateSandboxImageUI = (...args) => sandbox().updateSandboxImageUI?.(...args);
export const bindSandboxToggleEvents = (...args) => sandbox().bindSandboxToggleEvents?.(...args);
export const bindSandboxImageEvents = (...args) => sandbox().bindSandboxImageEvents?.(...args);
