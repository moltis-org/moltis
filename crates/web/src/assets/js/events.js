// ── Event bus (pub/sub for WebSocket events) ─────────────────
export var eventListeners = {};

export function onEvent(eventName, handler) {
	(eventListeners[eventName] = eventListeners[eventName] || []).push(handler);
	return function off() {
		var arr = eventListeners[eventName];
		if (arr) {
			var idx = arr.indexOf(handler);
			if (idx !== -1) arr.splice(idx, 1);
		}
	};
}
