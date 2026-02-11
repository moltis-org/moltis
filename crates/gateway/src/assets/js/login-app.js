import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { initTheme } from "./theme.js";

initTheme();

// Read identity from server-injected gon data (for emoji + name in title).
var gon = window.__MOLTIS__ || {};
var identity = gon.identity || null;

// Set page title from identity.
if (identity?.name) document.title = identity.name;

// ── Login form component ─────────────────────────────────────

function LoginApp() {
	var [password, setPassword] = useState("");
	var [error, setError] = useState(null);
	var [loading, setLoading] = useState(false);
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

	function onPasswordLogin(e) {
		e.preventDefault();
		setError(null);
		setLoading(true);
		fetch("/api/auth/login", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ password }),
		})
			.then((r) => {
				if (r.ok) {
					location.href = "/";
				} else {
					setError("Invalid password");
					setLoading(false);
				}
			})
			.catch((err) => {
				setError(err.message);
				setLoading(false);
			});
	}

	function onPasskeyLogin() {
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

	if (!ready) {
		return html`<div class="auth-card">
			<div class="text-sm text-[var(--muted)]">Loading\u2026</div>
		</div>`;
	}

	var title = (identity?.emoji ? `${identity.emoji} ` : "") + (identity?.name || "moltis");
	var showPassword = hasPassword || !hasPasskeys;
	var showPasskeys = hasPasskeys;
	var showDivider = showPassword && showPasskeys;

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
			<button type="submit" class="provider-btn w-full mt-1" disabled=${loading}>
				${loading ? "Signing in\u2026" : "Sign in"}
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
