// E2E test compatibility shim.
// The e2e helpers dynamically import this file to check WebSocket
// connection state. With Vite bundling, the real state module is
// inside the bundle, but we expose it on window.__moltis_state
// from main.tsx so this shim can re-export it.
//
// This file is NOT part of the Vite bundle — it's served as a
// static asset alongside the old share-app.mjs.

const S = window.__moltis_state || {};
export const connected = S.connected;
export const ws = S.ws;
export default S;
