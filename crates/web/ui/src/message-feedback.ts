// ── Thumbs up/down feedback on an assistant message ──────────
//
// Submits a "user-feedback" score against the trace that produced the
// message. The control only appears when instrumentation is actually
// collecting scores — a thumb that goes nowhere is worse than no thumb.

import { sendRpc } from "./helpers";
import { showToast } from "./ui";

export type FeedbackSignal = "positive" | "negative";

interface FeedbackStatus {
	enabled: boolean;
	instrumentation_active: boolean;
	retention_days: number;
}

let statusPromise: Promise<FeedbackStatus | null> | null = null;

/** Feedback availability, fetched once per page load. */
export function feedbackStatus(): Promise<FeedbackStatus | null> {
	if (!statusPromise) {
		statusPromise = sendRpc("feedback.status", {}).then((result) => {
			if (!result?.ok) return null;
			return (result.payload as FeedbackStatus | undefined) ?? null;
		});
	}
	return statusPromise;
}

/** Reset the cached status, for tests and after a settings change. */
export function resetFeedbackStatus(): void {
	statusPromise = null;
}

export interface FeedbackContext {
	sessionKey: string;
	runId?: string;
}

/**
 * Build the thumbs up/down pair, or `null` when there is nothing to attribute
 * a score to.
 *
 * A message with no run id predates the correlation table, so a thumb on it
 * could not be tied to a trace.
 */
export function buildFeedbackButtons(
	ctx: FeedbackContext,
	makeButton: (iconClass: string, title: string) => HTMLButtonElement,
): HTMLElement[] | null {
	if (!ctx.runId) return null;

	let active: FeedbackSignal | null = null;
	const up = makeButton("icon-thumbs-up", "Good response");
	const down = makeButton("icon-thumbs-down", "Bad response");

	const submit = async (signal: FeedbackSignal, button: HTMLButtonElement): Promise<void> => {
		// Clicking the active thumb withdraws the opinion rather than
		// re-asserting it.
		const clearing = active === signal;
		const result = await sendRpc("feedback.submit", {
			sessionKey: ctx.sessionKey,
			messageId: ctx.runId,
			signal: clearing ? "clear" : signal,
		});

		const payload = result?.payload as { ok?: boolean; outcome?: string } | undefined;
		if (!(result?.ok && payload?.ok)) {
			// Say why rather than failing silently: "unknown_message" means the
			// turn aged out of the retention window, which is not the user's
			// fault and not something a retry fixes.
			const reason =
				payload?.outcome === "unknown_message"
					? "This message is too old to rate"
					: payload?.outcome === "disabled"
						? "Feedback collection is turned off"
						: "Could not record feedback";
			showToast(reason, "error");
			return;
		}

		up.classList.remove("msg-action-btn-active");
		down.classList.remove("msg-action-btn-active");
		active = clearing ? null : signal;
		if (!clearing) button.classList.add("msg-action-btn-active");
		up.setAttribute("aria-pressed", String(active === "positive"));
		down.setAttribute("aria-pressed", String(active === "negative"));
	};

	up.setAttribute("aria-pressed", "false");
	down.setAttribute("aria-pressed", "false");
	up.addEventListener("click", () => void submit("positive", up));
	down.addEventListener("click", () => void submit("negative", down));

	return [up, down];
}
