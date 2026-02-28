// ── Bridge between imperative DOM and Preact RunDetail component ──

import { html } from "htm/preact";
import { render } from "preact";
import { RunDetail } from "./components/run-detail.js";

/**
 * Mount a RunDetail component inside a DOM element.
 * @param {HTMLElement} container - The parent element to render into
 * @param {string} sessionKey - Session key for RPC calls
 * @param {string} runId - The run ID to display details for
 */
export function mountRunDetail(container, sessionKey, runId) {
	var wrapper = document.createElement("div");
	wrapper.className = "run-detail-mount";
	container.appendChild(wrapper);
	render(html`<${RunDetail} sessionKey=${sessionKey} runId=${runId} />`, wrapper);
}
