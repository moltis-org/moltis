import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { applyIdentityFavicon, formatLoginTitle } from "./branding";
import { init as initI18n, t } from "./i18n";
import * as S from "./state";
import { initTheme } from "./theme";
import * as _wsConnect from "./ws-connect";

// Expose state module for E2E test WS mocking via shims.
window.__moltis_state = S;
window.__moltis_modules = { ...(window.__moltis_modules || {}), "ws-connect": _wsConnect };

// ── Types ────────────────────────────────────────────────────

interface IdentityInfo {
	emoji?: string;
	name?: string;
	[key: string]: unknown;
}

interface AuthStatus {
	authenticated?: boolean;
	has_passkeys?: boolean;
	has_password?: boolean;
}

interface LoginFailure {
	type: "retry" | "invalid_password" | "error";
	retryAfter?: number;
	message?: string;
}

interface LoginCardProps {
	title: string;
	showPassword: boolean;
	showPasskeys: boolean;
	showDivider: boolean;
	password: string;
	setPassword: (v: string) => void;
	onPasswordLogin: (e: Event) => void;
	onPasskeyLogin: () => void;
	loading: boolean;
	retrySecondsLeft: number;
	error: string | null;
}

// ── Init ─────────────────────────────────────────────────────

initTheme();
const i18nReady = initI18n().catch((err: unknown) => {
	console.warn("[i18n] login init failed", err);
});

// Read identity from server-injected gon data (name for title).
const gonData = (window as unknown as { __MOLTIS__?: Record<string, unknown> }).__MOLTIS__ || {};
const identity = (gonData.identity as IdentityInfo) || null;

// Set page branding from identity.
document.title = formatLoginTitle(identity);
applyIdentityFavicon(identity);
showVaultBanner((gonData.vault_status as string) || null);

function showVaultBanner(status: string | null): void {
	const el = document.getElementById("vaultBanner");
	if (!el) return;
	el.style.display = status === "sealed" ? "" : "none";
}

// ── Login failure parser ─────────────────────────────────────

async function parseLoginFailure(response: Response): Promise<LoginFailure> {
	if (response.status === 429) {
		let retryAfter = 0;
		try {
			const data = await response.json();
			if (data && Number.isFinite(data.retry_after_seconds)) {
				retryAfter = Math.max(1, Math.ceil(data.retry_after_seconds));
			}
		} catch {
			// Ignore JSON parse errors and fall back to Retry-After header.
		}
		if (retryAfter <= 0) {
			const retryAfterHeader = Number.parseInt(response.headers.get("Retry-After") || "0", 10);
			if (Number.isFinite(retryAfterHeader) && retryAfterHeader > 0) {
				retryAfter = retryAfterHeader;
			}
		}
		return { type: "retry", retryAfter: Math.max(1, retryAfter) };
	}

	if (response.status === 401) {
		return { type: "invalid_password" };
	}

	const bodyText = await response.text();
	return { type: "error", message: bodyText || t("login:loginFailed") };
}

// ── Passkey helpers ──────────────────────────────────────────

function startPasskeyLogin(setError: (v: string | null) => void, setLoading: (v: boolean) => void): void {
	setError(null);
	if (/^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[")) {
		setError(t("login:passkeyRequiresDomain", { hostname: location.hostname }));
		return;
	}
	setLoading(true);
	fetch("/api/auth/passkey/auth/begin", { method: "POST" })
		.then((r) => r.json())
		.then(
			(data: {
				options: PublicKeyCredentialRequestOptions & {
					publicKey: { challenge: string; allowCredentials?: Array<{ id: string }> };
				};
				challenge_id: string;
			}) => {
				const options = data.options;
				(options.publicKey as unknown as { challenge: ArrayBuffer }).challenge = base64ToBuffer(
					options.publicKey.challenge as unknown as string,
				);
				if (options.publicKey.allowCredentials) {
					for (const c of options.publicKey.allowCredentials) {
						(c as unknown as { id: ArrayBuffer }).id = base64ToBuffer(c.id as unknown as string);
					}
				}
				return navigator.credentials
					.get({ publicKey: options.publicKey as unknown as PublicKeyCredentialRequestOptions })
					.then((cred) => ({ cred: cred as PublicKeyCredential, challengeId: data.challenge_id }));
			},
		)
		.then(({ cred, challengeId }) => {
			const assertionResponse = cred.response as AuthenticatorAssertionResponse;
			const body = {
				challenge_id: challengeId,
				credential: {
					id: cred.id,
					rawId: bufferToBase64(cred.rawId),
					type: cred.type,
					response: {
						authenticatorData: bufferToBase64(assertionResponse.authenticatorData),
						clientDataJSON: bufferToBase64(assertionResponse.clientDataJSON),
						signature: bufferToBase64(assertionResponse.signature),
						userHandle: assertionResponse.userHandle ? bufferToBase64(assertionResponse.userHandle) : null,
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
				return r.text().then((msg: string) => {
					setError(msg || t("login:passkeyAuthFailed"));
					setLoading(false);
				});
			}
		})
		.catch((err: Error) => {
			setError(err.message || t("login:passkeyAuthFailed"));
			setLoading(false);
		});
}

// ── Login card renderer ──────────────────────────────────────

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
}: LoginCardProps): VNode {
	return (
		<div className="auth-card">
			<h1 className="auth-title">{title}</h1>
			<p className="auth-subtitle">{t("login:signInToContinue")}</p>
			{showPassword ? (
				<form onSubmit={onPasswordLogin} className="flex flex-col gap-3">
					<div>
						<label className="text-xs text-[var(--muted)] mb-1 block">{t("login:password")}</label>
						<input
							type="password"
							className="provider-key-input w-full"
							value={password}
							onInput={(e) => setPassword((e.target as HTMLInputElement).value)}
							placeholder={t("login:enterPassword")}
							autofocus
						/>
					</div>
					<button type="submit" className="provider-btn w-full mt-1" disabled={loading || retrySecondsLeft > 0}>
						{loading
							? t("login:signingIn")
							: retrySecondsLeft > 0
								? t("login:retryIn", { seconds: retrySecondsLeft })
								: t("login:signIn")}
					</button>
				</form>
			) : null}
			{showDivider ? (
				<div className="auth-divider">
					<span>{t("login:or")}</span>
				</div>
			) : null}
			{showPasskeys ? (
				<button
					type="button"
					className={`provider-btn ${showPassword ? "provider-btn-secondary" : ""} w-full`}
					onClick={onPasskeyLogin}
					disabled={loading}
				>
					{t("login:signInWithPasskey")}
				</button>
			) : null}
			{error ? <p className="auth-error mt-2">{error}</p> : null}
		</div>
	);
}

// ── Login form component ─────────────────────────────────────

function LoginApp(): VNode {
	const [password, setPassword] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);
	const [retrySecondsLeft, setRetrySecondsLeft] = useState(0);
	const [hasPasskeys, setHasPasskeys] = useState(false);
	const [hasPassword, setHasPassword] = useState(false);
	const [ready, setReady] = useState(false);

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((data: AuthStatus | null) => {
				if (!data) return;
				if (data.authenticated) {
					location.href = "/";
					return;
				}
				setHasPasskeys(!!data.has_passkeys);
				setHasPassword(!!data.has_password);
				setReady(true);
			})
			.catch(() => setReady(true));
	}, []);

	useEffect(() => {
		if (retrySecondsLeft <= 0) return undefined;
		const timer = setInterval(() => {
			setRetrySecondsLeft((value) => (value > 1 ? value - 1 : 0));
		}, 1000);
		return () => clearInterval(timer);
	}, [retrySecondsLeft]);

	function onPasswordLogin(e: Event): void {
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

				const failure = await parseLoginFailure(r);
				if (failure.type === "retry") {
					setRetrySecondsLeft(failure.retryAfter || 1);
					setError(t("login:wrongPassword"));
				} else if (failure.type === "invalid_password") {
					setError(t("login:invalidPassword"));
				} else {
					setError(failure.message || null);
				}
				setLoading(false);
			})
			.catch((err: Error) => {
				setError(err.message);
				setLoading(false);
			});
	}

	function onPasskeyLogin(): void {
		startPasskeyLogin(setError, setLoading);
	}

	if (!ready) {
		return (
			<div className="auth-card">
				<div className="text-sm text-[var(--muted)]">{t("common:status.loading")}</div>
			</div>
		);
	}

	const title = formatLoginTitle(identity);
	const showPassword = hasPassword || !hasPasskeys;
	const showPasskeys = hasPasskeys;
	const showDivider = showPassword && showPasskeys;

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

function base64ToBuffer(b64: string): ArrayBuffer {
	let str = b64.replace(/-/g, "+").replace(/_/g, "/");
	while (str.length % 4) str += "=";
	const bin = atob(str);
	const buf = new Uint8Array(bin.length);
	for (let i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
	return buf.buffer;
}

function bufferToBase64(buf: ArrayBuffer): string {
	const bytes = new Uint8Array(buf);
	let str = "";
	for (const b of bytes) str += String.fromCharCode(b);
	return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

// ── Mount ────────────────────────────────────────────────────

const root = document.getElementById("loginRoot");
if (root) {
	i18nReady.finally(() => {
		render(<LoginApp />, root);
	});
}
