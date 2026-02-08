import { nextId } from "./helpers.js";
import { mountOnboarding } from "./onboarding-view.js";
import * as S from "./state.js";
import { initTheme, injectMarkdownStyles } from "./theme.js";

function connectRpc() {
	var protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
	var url = `${protocol}//${window.location.host}/ws`;
	var ws = new WebSocket(url);
	S.setWs(ws);

	ws.onopen = () => {
		var id = nextId();
		S.pending[id] = (frame) => {
			var hello = frame?.ok && frame.payload;
			if (hello?.type === "hello-ok") {
				S.setConnected(true);
				S.setReconnectDelay(1000);
				return;
			}
			// Invalid handshake response, close and retry.
			S.setConnected(false);
			ws.close();
		};
		ws.send(
			JSON.stringify({
				type: "req",
				id: id,
				method: "connect",
				params: {
					minProtocol: 3,
					maxProtocol: 3,
					client: {
						id: "web-chat-ui",
						version: "0.1.0",
						platform: "browser",
						mode: "operator",
					},
				},
			}),
		);
	};

	ws.onmessage = (event) => {
		var msg;
		try {
			msg = JSON.parse(event.data);
		} catch {
			return;
		}
		if (msg.type === "res" && msg.id && S.pending[msg.id]) {
			S.pending[msg.id](msg);
			delete S.pending[msg.id];
		}
	};

	ws.onclose = () => {
		S.setConnected(false);
		S.setWs(null);
		for (var id in S.pending) {
			S.pending[id]({ ok: false, error: { message: "WebSocket disconnected" } });
			delete S.pending[id];
		}
		var delay = S.reconnectDelay;
		window.setTimeout(connectRpc, delay);
		S.setReconnectDelay(Math.min(delay * 2, 10000));
	};

	ws.onerror = () => {
		// Handled by close/reconnect path.
	};
}

initTheme();
injectMarkdownStyles();
connectRpc();

var root = document.getElementById("onboardingRoot");
if (root) {
	mountOnboarding(root);
}
