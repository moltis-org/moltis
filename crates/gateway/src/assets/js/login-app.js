import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { formatLoginTitle } from "./branding.js";
import { initTheme } from "./theme.js";

initTheme();

// Read identity from server-injected gon data (name for title).
var gon = window.__MOLTIS__ || {};
var identity = gon.identity || null;

// Set page branding from identity.
document.title = formatLoginTitle(identity);

async function parseLoginFailure(response) {
	if (response.status === 429) {
		var retryAfter = 0;
		try {
			var data = await response.json();
			if (data && Number.isFinite(data.retry_after_seconds)) {
				retryAfter = Math.max(1, Math.ceil(data.retry_after_seconds));
			}
		} catch {
			// Ignore JSON parse errors and fall back to Retry-After header.
		}
		if (retryAfter <= 0) {
			var retryAfterHeader = Number.parseInt(response.headers.get("Retry-After") || "0", 10);
			if (Number.isFinite(retryAfterHeader) && retryAfterHeader > 0) {
				retryAfter = retryAfterHeader;
			}
		}
		return { type: "retry", retryAfter: Math.max(1, retryAfter) };
	}

	if (response.status === 401) {
		return { type: "invalid_password" };
	}

	var bodyText = await response.text();
	return { type: "error", message: bodyText || "Login failed" };
}

function startPasskeyLogin(setError, setLoading) {
	setError(null);
	if (/^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[")) {
		setError(`Passkeys require a domain name. Use localhost instead of ${location.hostname}`);
		return;
	}
	setLoading(true);
	fetch("/api/auth/passkey/auth/begin", { method: "POST" })
		.then((r) => r.json())
		.then((data) => {
			var options = data.options;
			options.publicKey.challenge = base64ToBuffer(options.publicKey.challenge);
			if (options.publicKey.allowCredentials) {
				for (var c of options.publicKey.allowCredentials) {
					c.id = base64ToBuffer(c.id);
				}
			}
			return navigator.credentials
				.get({ publicKey: options.publicKey })
				.then((cred) => ({ cred, challengeId: data.challenge_id }));
		})
		.then(({ cred, challengeId }) => {
			var body = {
				challenge_id: challengeId,
				credential: {
					id: cred.id,
					rawId: bufferToBase64(cred.rawId),
					type: cred.type,
					response: {
						authenticatorData: bufferToBase64(cred.response.authenticatorData),
						clientDataJSON: bufferToBase64(cred.response.clientDataJSON),
						signature: bufferToBase64(cred.response.signature),
						userHandle: cred.response.userHandle ? bufferToBase64(cred.response.userHandle) : null,
					},
				},
			};
			return fetch("/api/auth/passkey/auth/finish", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify(body),
			});
		})
		.then((r) => {
			if (r.ok) {
				location.href = "/";
			} else {
				return r.text().then((t) => {
					setError(t || "Passkey authentication failed");
					setLoading(false);
				});
			}
		})
		.catch((err) => {
			setError(err.message || "Passkey authentication failed");
			setLoading(false);
		});
}

function renderLoginCard({
	title,
	showPassword,
	showPasskeys,
	showDivider,
	password,
	setPassword,
	onPasswordLogin,
	onPasskeyLogin,
	loading,
	retrySecondsLeft,
	error,
}) {
	return html`<div class="auth-card">
		<h1 class="auth-title">${title}</h1>
		<p class="auth-subtitle">Sign in to continue</p>
		${
			showPassword
				? html`<form onSubmit=${onPasswordLogin} class="flex flex-col gap-3">
			<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Password</label>
				<input
					type="password"
					class="provider-key-input w-full"
					value=${password}
					onInput=${(e) => setPassword(e.target.value)}
					placeholder="Enter password"
					autofocus
				/>
			</div>
			<button type="submit" class="provider-btn w-full mt-1" disabled=${loading || retrySecondsLeft > 0}>
				${loading ? "Signing in\u2026" : retrySecondsLeft > 0 ? `Retry in ${retrySecondsLeft}s` : "Sign in"}
			</button>
		</form>`
				: null
		}
		${showDivider ? html`<div class="auth-divider"><span>or</span></div>` : null}
		${
			showPasskeys
				? html`<button
				type="button"
				class="provider-btn ${showPassword ? "provider-btn-secondary" : ""} w-full"
				onClick=${onPasskeyLogin}
				disabled=${loading}
			>
				Sign in with passkey
			</button>`
				: null
		}
		${error ? html`<p class="auth-error mt-2">${error}</p>` : null}
	</div>`;
}

// ── Login form component ─────────────────────────────────────

function LoginApp() {
	var [password, setPassword] = useState("");
	var [error, setError] = useState(null);
	var [loading, setLoading] = useState(false);
	var [retrySecondsLeft, setRetrySecondsLeft] = useState(0);
	var [hasPasskeys, setHasPasskeys] = useState(false);
	var [hasPassword, setHasPassword] = useState(false);
	var [ready, setReady] = useState(false);

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((data) => {
				if (!data) return;
				if (data.authenticated) {
					location.href = "/";
					return;
				}
				setHasPasskeys(data.has_passkeys);
				setHasPassword(data.has_password);
				setReady(true);
			})
			.catch(() => setReady(true));
	}, []);

	useEffect(() => {
		if (retrySecondsLeft <= 0) return undefined;
		var timer = setInterval(() => {
			setRetrySecondsLeft((value) => (value > 1 ? value - 1 : 0));
		}, 1000);
		return () => clearInterval(timer);
	}, [retrySecondsLeft]);

	function onPasswordLogin(e) {
		e.preventDefault();
		if (retrySecondsLeft > 0) return;
		setError(null);
		setLoading(true);
		fetch("/api/auth/login", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ password }),
		})
			.then(async (r) => {
				if (r.ok) {
					location.href = "/";
					return;
				}

				var failure = await parseLoginFailure(r);
				if (failure.type === "retry") {
					setRetrySecondsLeft(failure.retryAfter);
					setError("Wrong password");
				} else if (failure.type === "invalid_password") {
					setError("Invalid password");
				} else {
					setError(failure.message);
				}
				setLoading(false);
			})
			.catch((err) => {
				setError(err.message);
				setLoading(false);
			});
	}

	function onPasskeyLogin() {
		startPasskeyLogin(setError, setLoading);
	}

	if (!ready) {
		return html`<div class="auth-card">
			<div class="text-sm text-[var(--muted)]">Loading\u2026</div>
		</div>`;
	}

	var title = formatLoginTitle(identity);
	var showPassword = hasPassword || !hasPasskeys;
	var showPasskeys = hasPasskeys;
	var showDivider = showPassword && showPasskeys;

	return renderLoginCard({
		title,
		showPassword,
		showPasskeys,
		showDivider,
		password,
		setPassword,
		onPasswordLogin,
		onPasskeyLogin,
		loading,
		retrySecondsLeft,
		error,
	});
}

// ── Base64url helpers for WebAuthn ───────────────────────────

function base64ToBuffer(b64) {
	var str = b64.replace(/-/g, "+").replace(/_/g, "/");
	while (str.length % 4) str += "=";
	var bin = atob(str);
	var buf = new Uint8Array(bin.length);
	for (var i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
	return buf.buffer;
}

function bufferToBase64(buf) {
	var bytes = new Uint8Array(buf);
	var str = "";
	for (var b of bytes) str += String.fromCharCode(b);
	return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

// ── Mount ────────────────────────────────────────────────────

var root = document.getElementById("loginRoot");
if (root) {
	render(html`<${LoginApp} />`, root);
}
