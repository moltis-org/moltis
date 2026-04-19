// ── Shared channel RPC wrappers and validation ────────────────
//
// Used by page-channels.js and onboarding-view.js.

import { get as getGon } from "./gon";
import { sendRpc } from "./helpers";
import { ChannelType } from "./types";

export const MATRIX_DOCS_URL = "https://docs.moltis.org/matrix.html";
export const MATRIX_DEFAULT_HOMESERVER = "https://matrix.org";
export const MATRIX_ENCRYPTION_GUIDANCE =
	"Encrypted Matrix chats require OIDC or Password auth. Access token auth can connect for plain Matrix traffic, but it reuses an existing Matrix session without that device's private encryption keys, so Moltis cannot reliably decrypt encrypted chats. Use OIDC (recommended) or Password so Moltis creates and persists its own Matrix device keys, then finish Element verification in the same Matrix DM or room by sending `verify yes`, `verify no`, `verify show`, or `verify cancel` as normal chat messages.";

export function matrixAuthModeGuidance(authMode: string | undefined): string {
	const mode = normalizeMatrixAuthMode(authMode);
	if (mode === "oidc")
		return "Recommended for homeservers using Matrix Authentication Service (e.g. matrix.org since April 2025). Moltis authenticates via your browser \u2014 no password or token needed.";
	if (mode === "password")
		return "Required for encrypted Matrix chats. Moltis logs in as its own Matrix device and stores the device's encryption keys locally.";
	return "Does not support encrypted Matrix chats. Access tokens authenticate an existing Matrix session, but they do not transfer that device's private encryption keys into Moltis.";
}

export function channelStorageNote(): string {
	const dbPath = String(getGon("channel_storage_db_path") || "").trim();
	if (dbPath) {
		return `Channels added or edited in the web UI are stored in Moltis's internal database (${dbPath}). They are not written back to moltis.toml. The channel picker itself comes from [channels].offered in moltis.toml, so reload this page after editing that list.`;
	}
	return "Channels added or edited in the web UI are stored in Moltis's internal database (moltis.db). They are not written back to moltis.toml. The channel picker itself comes from [channels].offered in moltis.toml, so reload this page after editing that list.";
}

interface ValidationSuccess {
	valid: true;
}

interface ValidationFailure {
	valid: false;
	error: string;
}

type ValidationResult = ValidationSuccess | ValidationFailure;

interface ValidateChannelOptions {
	matrixAuthMode?: string;
	matrixUserId?: string;
}

/**
 * Validate required channel fields before submission.
 */
export function validateChannelFields(
	type: string,
	accountId: string,
	credential: string,
	options: ValidateChannelOptions = {},
): ValidationResult {
	if (!accountId.trim()) {
		return { valid: false, error: "Account ID is required." };
	}
	if (!credential.trim() && normalizeMatrixAuthMode(options.matrixAuthMode) !== "oidc") {
		if (type === ChannelType.Matrix) {
			return { valid: false, error: matrixCredentialError(options.matrixAuthMode) };
		}
		return {
			valid: false,
			error: type === ChannelType.MsTeams ? "App password is required." : "Bot token is required.",
		};
	}
	if (
		type === ChannelType.Matrix &&
		normalizeMatrixAuthMode(options.matrixAuthMode) === "password" &&
		!String(options.matrixUserId || "").trim()
	) {
		return { valid: false, error: "Matrix user ID is required for password login." };
	}
	return { valid: true };
}

export function normalizeMatrixAuthMode(authMode: string | undefined): string {
	if (authMode === "oidc") return "oidc";
	if (authMode === "password") return "password";
	return "access_token";
}

export function normalizeMatrixOwnershipMode(mode: string | undefined): string {
	return mode === "moltis_owned" ? "moltis_owned" : "user_managed";
}

export function matrixOwnershipModeGuidance(authMode: string | undefined, ownershipMode: string | undefined): string {
	const mode = normalizeMatrixAuthMode(authMode);
	if (mode !== "password" && mode !== "oidc") {
		return "Access token auth always stays user-managed because it reuses an existing Matrix session instead of giving Moltis full control of the account's encryption state.";
	}
	return normalizeMatrixOwnershipMode(ownershipMode) === "moltis_owned"
		? "Recommended for dedicated bot accounts. Moltis bootstraps cross-signing and recovery for this account so it can verify its own Matrix device automatically."
		: "Use this if you want to open the same bot account in Element or another Matrix client yourself. Moltis will not try to take over the account's cross-signing or recovery state.";
}

export function matrixCredentialLabel(authMode: string | undefined): string {
	return normalizeMatrixAuthMode(authMode) === "password" ? "Password" : "Access Token";
}

export function matrixCredentialPlaceholder(authMode: string | undefined): string {
	return normalizeMatrixAuthMode(authMode) === "password" ? "Account password" : "syt_...";
}

export function matrixCredentialError(authMode: string | undefined): string {
	return normalizeMatrixAuthMode(authMode) === "password" ? "Password is required." : "Access token is required.";
}

function randomSuffix(length: number): string {
	if (typeof window !== "undefined" && window.crypto?.getRandomValues) {
		const bytes = new Uint8Array(length);
		window.crypto.getRandomValues(bytes);
		return Array.from(bytes, (byte) => (byte % 36).toString(36)).join("");
	}
	let value = "";
	while (value.length < length) {
		value += Math.floor(Math.random() * 36).toString(36);
	}
	return value.slice(0, length);
}

function slugifyMatrixAccountPart(value: string | undefined): string {
	return String(value || "")
		.toLowerCase()
		.trim()
		.replace(/^@/, "")
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/-+/g, "-")
		.replace(/^-|-$/g, "");
}

function matrixHomeserverHost(homeserver: string | undefined): string {
	let raw = String(homeserver || "").trim();
	if (!raw) return "";
	if (!/^https?:\/\//i.test(raw)) raw = `https://${raw}`;
	try {
		return new URL(raw).hostname;
	} catch (_error) {
		return "";
	}
}

interface DeriveMatrixAccountIdOptions {
	userId?: string;
	homeserver?: string;
}

/**
 * Generate a local Matrix account identifier for Moltis.
 * Prefer the Matrix user ID when present, otherwise derive from homeserver.
 */
export function deriveMatrixAccountId(options: DeriveMatrixAccountIdOptions = {}): string {
	const userSlug = slugifyMatrixAccountPart(options.userId);
	if (userSlug) return userSlug.slice(0, 80);

	const hostSlug = slugifyMatrixAccountPart(matrixHomeserverHost(options.homeserver));
	const base = hostSlug || "matrix";
	return `${base}-${randomSuffix(6)}`.slice(0, 80);
}

/**
 * Normalize Matrix OTP cooldown input to a positive integer.
 */
export function normalizeMatrixOtpCooldown(value: string | number | null | undefined, fallback: number = 300): number {
	const parsed = Number.parseInt(String(value || ""), 10);
	return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

interface ParseOk {
	ok: true;
	value: Record<string, unknown>;
}

interface ParseError {
	ok: false;
	error: string;
}

type ParseResult = ParseOk | ParseError;

/**
 * Parse an optional advanced channel config JSON object.
 */
export function parseChannelConfigPatch(text: string | null | undefined): ParseResult {
	const raw = String(text || "").trim();
	if (!raw) return { ok: true, value: {} };
	try {
		const value: unknown = JSON.parse(raw);
		if (!(value && typeof value === "object" && !Array.isArray(value))) {
			return { ok: false, error: "Advanced config must be a JSON object." };
		}
		return { ok: true, value: value as Record<string, unknown> };
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error || "unknown error");
		return { ok: false, error: `Advanced config JSON is invalid: ${message}` };
	}
}

/**
 * Add a new channel (e.g. Telegram bot).
 */
export function addChannel(type: string, accountId: string, config: Record<string, unknown>): Promise<unknown> {
	return sendRpc("channels.add", { type, account_id: accountId, config });
}

/**
 * Fetch the current status of all configured channels.
 * Resolves with the RPC response; payload has `{ channels: [] }`.
 */
export function fetchChannelStatus(): Promise<unknown> {
	return sendRpc("channels.status", {});
}

/**
 * Default base URL for Teams webhook endpoints.
 * Prefer a discovered public URL when available, otherwise fall back to the
 * current page origin.
 */
export function defaultTeamsBaseUrl(preferredPublicUrl?: string): string {
	const preferred = (preferredPublicUrl || "").trim();
	if (preferred) return preferred;
	if (typeof window === "undefined") return "";
	return window.location?.origin || "";
}

/**
 * Normalise a user-provided base URL into `protocol://host`.
 */
export function normalizeBaseUrlForWebhook(baseUrl: string | undefined): string {
	let raw = (baseUrl || "").trim();
	if (!raw) raw = defaultTeamsBaseUrl();
	if (!raw) return "";
	if (!/^https?:\/\//i.test(raw)) raw = `https://${raw}`;
	try {
		const parsed = new URL(raw);
		return `${parsed.protocol}//${parsed.host}`;
	} catch (_e) {
		return "";
	}
}

/**
 * Generate a random 48-hex-char webhook secret.
 */
export function generateWebhookSecretHex(): string {
	if (typeof window !== "undefined" && window.crypto?.getRandomValues) {
		const bytes = new Uint8Array(24);
		window.crypto.getRandomValues(bytes);
		return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
	}
	let value = "";
	while (value.length < 48) {
		value += Math.floor(Math.random() * 16).toString(16);
	}
	return value.slice(0, 48);
}

/**
 * Build the full Teams messaging endpoint URL.
 */
export function buildTeamsEndpoint(
	baseUrl: string | undefined,
	accountId: string | undefined,
	webhookSecret: string | undefined,
): string {
	const normalizedBase = normalizeBaseUrlForWebhook(baseUrl);
	const account = (accountId || "").trim();
	const secret = (webhookSecret || "").trim();
	if (!(normalizedBase && account && secret)) return "";
	return `${normalizedBase}/api/channels/msteams/${encodeURIComponent(account)}/webhook?secret=${encodeURIComponent(secret)}`;
}
