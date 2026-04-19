// E2E test compatibility shim.
// The e2e helpers dynamically import this file to check and control
// WebSocket connection state. With Vite bundling, the real state
// module is inside the bundle, but we expose it on window.__moltis_state
// from main.tsx so this shim can proxy to it.
//
// This file is NOT part of the Vite bundle — it's served as a
// static asset alongside the old share-app.mjs.

const S = window.__moltis_state || {};

// Read-only state
export const connected = S.connected;
export const ws = S.ws;
export const pending = S.pending;

// Setters used by e2e tests to mock WebSocket state
export function setConnected(v) { if (S.setConnected) S.setConnected(v); }
export function setWs(v) { if (S.setWs) S.setWs(v); }

export default S;
