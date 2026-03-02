// ── Logs alert dot ──────────────────────────────────────────

import { sendRpc } from "./helpers.js";
import * as S from "./state.js";

var logsAlertDot = document.getElementById("logsAlertDot");

export function updateLogsAlert() {
	if (!logsAlertDot) return;
	if (S.unseenErrors > 0) {
		logsAlertDot.style.display = "";
		logsAlertDot.style.background = "var(--error)";
	} else if (S.unseenWarns > 0) {
		logsAlertDot.style.display = "";
		logsAlertDot.style.background = "var(--warn)";
	} else {
		logsAlertDot.style.display = "none";
	}
}

export function clearLogsAlert() {
	S.setUnseenErrors(0);
	S.setUnseenWarns(0);
	updateLogsAlert();
	if (S.connected) sendRpc("logs.ack", {});
}
