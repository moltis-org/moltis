// ── Non-chat event handlers ──────────────────────────────────

import { chatAddErrorMsg, renderApprovalCard } from "../chat-ui";
import { sendRpc } from "../helpers";
import { updateLogsAlert } from "../logs-alert";
import { fetchModels } from "../models";
import { currentPage, currentPrefix } from "../router";
import * as S from "../state";
import type {
	ApprovalPayload,
	AuthCredentialsPayload,
	LocationRequestPayload,
	LogEntryPayload,
	ModelsUpdatedPayload,
	WsErrorPayload,
} from "../types/ws-events";

export function handleApprovalEvent(payload: ApprovalPayload): void {
	renderApprovalCard(payload.requestId, payload.command);
}

export function handleLogEntry(payload: LogEntryPayload): void {
	if (S.logsEventHandler) S.logsEventHandler(payload);
	if (currentPage !== "/logs") {
		const ll = (payload.level || "").toUpperCase();
		if (ll === "ERROR") {
			S.setUnseenErrors(S.unseenErrors + 1);
			updateLogsAlert();
		} else if (ll === "WARN") {
			S.setUnseenWarns(S.unseenWarns + 1);
			updateLogsAlert();
		}
	}
}

export function handleWsError(payload: WsErrorPayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	chatAddErrorMsg(payload.message || "Unknown error");
}

export function handleLocationRequest(payload: LocationRequestPayload): void {
	const requestId = payload.requestId;
	if (!requestId) return;

	if (!navigator.geolocation) {
		sendRpc("location.result", {
			requestId,
			error: { code: 0, message: "Geolocation not supported" },
		});
		return;
	}

	// Coarse: city-level, fast, longer cache. Precise: GPS-level, fresh.
	const coarse = payload.precision === "coarse";
	const geoOpts: PositionOptions = coarse
		? { enableHighAccuracy: false, timeout: 10000, maximumAge: 1800000 }
		: { enableHighAccuracy: true, timeout: 15000, maximumAge: 60000 };

	navigator.geolocation.getCurrentPosition(
		(pos: GeolocationPosition) => {
			sendRpc("location.result", {
				requestId,
				location: {
					latitude: pos.coords.latitude,
					longitude: pos.coords.longitude,
					accuracy: pos.coords.accuracy,
				},
			});
		},
		(err: GeolocationPositionError) => {
			sendRpc("location.result", {
				requestId,
				error: { code: err.code, message: err.message },
			});
		},
		geoOpts,
	);
}

export function handleNetworkAuditEntry(payload: unknown): void {
	if (S.networkAuditEventHandler) S.networkAuditEventHandler(payload);
}

export function handleAuthCredentialsChanged(payload: AuthCredentialsPayload): void {
	if (payload?.reason === "password_changed" && window.__moltisSuppressNextPasswordChangedRedirect === true) {
		window.__moltisSuppressNextPasswordChangedRedirect = false;
		console.info("Deferring redirect for password_changed to show recovery key first");
		return;
	}
	console.warn("Auth credentials changed:", payload.reason);
	window.location.href = "/login";
}

let modelsUpdatedTimer: ReturnType<typeof setTimeout> | null = null;
export function handleModelsUpdated(payload: ModelsUpdatedPayload): void {
	// Progress/status frames are consumed directly by the Providers page.
	// Avoid spamming model refresh requests while a probe is running.
	if (payload?.phase === "start" || payload?.phase === "progress") return;
	if (modelsUpdatedTimer) return;
	modelsUpdatedTimer = setTimeout(() => {
		modelsUpdatedTimer = null;
		// fetchModels() delegates to modelStore.fetch() internally
		fetchModels();
		if (S.refreshProvidersPage) S.refreshProvidersPage();
	}, 150);
}
