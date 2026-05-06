// ── Bridge between imperative DOM and Preact RunDetail component ──

import { render } from "preact";
import { RunDetail } from "./components/RunDetail";

/**
 * Mount a RunDetail component inside a DOM element.
 * @param container - The parent element to render into
 * @param sessionKey - Session key for RPC calls
 * @param runId - The run ID to display details for
 */
export function mountRunDetail(container: HTMLElement, sessionKey: string, runId: string): void {
	const wrapper = document.createElement("div");
	wrapper.className = "run-detail-mount";
	container.appendChild(wrapper);
	render(<RunDetail sessionKey={sessionKey} runId={runId} />, wrapper);
}
