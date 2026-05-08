// ── Channel form sub-components for onboarding ───────────────
//
// Shared helpers and simple channel forms (Telegram, Discord, Nostr).
// Complex forms (Matrix, WhatsApp, Slack, Teams) live in ChannelStep.tsx.

import type { VNode } from "preact";
import { useState } from "preact/hooks";
import {
	addChannel,
	channelStorageNote,
	deriveSignalAccountId,
	parseChannelConfigPatch,
	validateChannelFields,
} from "../../channel-utils";
import { targetValue } from "../../typed-events";
import { ErrorPanel } from "../shared";

// ── Types ───────────────────────────────────────────────────

export interface ChannelFormProps {
	onConnected: (name: string, type: string) => void;
	error: string | null;
	setError: (e: string | null) => void;
}

// ── Shared components ───────────────────────────────────────

export function ChannelStorageNotice(): VNode {
	return (
		<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)]">
			<span className="font-medium text-[var(--text-strong)]">Storage note.</span> {channelStorageNote()}
		</div>
	);
}

interface AdvancedConfigPatchFieldProps {
	value: string;
	onInput: (v: string) => void;
}

export function AdvancedConfigPatchField({ value, onInput }: AdvancedConfigPatchFieldProps): VNode {
	return (
		<details className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3">
			<summary className="cursor-pointer text-xs font-medium text-[var(--text-strong)]">Advanced Config JSON</summary>
			<div className="mt-3 flex flex-col gap-2">
				<div className="text-xs text-[var(--muted)]">
					Optional JSON object merged on top of the form before save. Use this for channel-specific settings that do not
					have dedicated fields yet.
				</div>
				<div>
					<label className="text-xs text-[var(--muted)] mb-1 block">Advanced config JSON patch (optional)</label>
					<textarea
						name="channel_advanced_config"
						className="provider-key-input w-full min-h-[140px] font-mono text-xs"
						value={value}
						onInput={(e) => onInput(targetValue(e))}
						placeholder={'{"reply_to_message": true}'}
					/>
				</div>
			</div>
		</details>
	);
}

// ── Channel type selector ───────────────────────────────────

interface ChannelTypeSelectorProps {
	onSelect: (type: string) => void;
	offered: Set<string>;
}

export function ChannelTypeSelector({ onSelect, offered }: ChannelTypeSelectorProps): VNode {
	const channelOptions: [string, string, string][] = (
		[
			["telegram", "icon-telegram", "Telegram"],
			["whatsapp", "icon-whatsapp", "WhatsApp"],
			["msteams", "icon-msteams", "Microsoft Teams"],
			["discord", "icon-discord", "Discord"],
			["slack", "icon-slack", "Slack"],
			["matrix", "icon-matrix", "Matrix"],
			["nostr", "icon-nostr", "Nostr"],
			["signal", "icon-signal", "Signal"],
		] as [string, string, string][]
	).filter(([type]) => offered.has(type));

	return (
		<div className="grid grid-cols-2 gap-3 md:grid-cols-3" data-testid="channel-type-selector">
			{channelOptions.map(([type, iconClass, label]) => (
				<button
					key={type}
					type="button"
					className="backend-card w-full min-h-[120px] items-center justify-center gap-4 px-4 py-8 text-center"
					onClick={() => onSelect(type)}
				>
					<span className={`icon icon-xl ${iconClass}`} />
					<span className="text-sm font-medium text-[var(--text-strong)]">{label}</span>
				</button>
			))}
		</div>
	);
}

// ── Channel success display ─────────────────────────────────

export function channelDisplayLabel(type: string): string {
	if (type === "msteams") return "Microsoft Teams";
	if (type === "discord") return "Discord";
	if (type === "slack") return "Slack";
	if (type === "whatsapp") return "WhatsApp";
	if (type === "matrix") return "Matrix";
	if (type === "nostr") return "Nostr";
	if (type === "signal") return "Signal";
	return "Telegram";
}

export function ChannelSuccess({
	channelName,
	channelType: type,
	onAnother,
}: {
	channelName: string;
	channelType: string;
	onAnother: () => void;
}): VNode {
	const label = channelDisplayLabel(type);
	return (
		<div className="flex flex-col gap-3">
			<div className="rounded-md border border-[var(--ok)] bg-[var(--surface)] p-4 flex gap-3 items-center">
				<span className="icon icon-lg icon-check-circle shrink-0" style="color:var(--ok)" />
				<div>
					<div className="text-sm font-medium text-[var(--text-strong)]">Channel connected</div>
					<div className="text-xs text-[var(--muted)] mt-0.5">
						{channelName} ({label}) is now linked to your agent.
					</div>
				</div>
			</div>
			{type === "discord" && (
				<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1.5">
					<span className="font-medium text-[var(--text-strong)]">Next steps</span>
					<span>
						&bull; <strong>Invite to a server:</strong> the invite link was shown on the previous screen. You can also
						generate one in the{" "}
						<a
							href="https://discord.com/developers/applications"
							target="_blank"
							rel="noopener"
							className="text-[var(--accent)] underline"
						>
							Developer Portal
						</a>{" "}
						&rarr; OAuth2 &rarr; URL Generator (scope: bot, permissions: Send Messages, Attach Files, Read Message
						History).
					</span>
					<span>
						&bull; <strong>DM the bot:</strong> search for the bot&rsquo;s username in Discord and click Message. Make
						sure your username is in the DM allowlist.
					</span>
					<span>
						&bull; <strong>In a server:</strong> @mention the bot to get a response.
					</span>
				</div>
			)}
			<button
				type="button"
				className="text-xs text-[var(--accent)] cursor-pointer bg-transparent border-none underline self-start"
				onClick={onAnother}
			>
				Connect another channel
			</button>
		</div>
	);
}

// ── Telegram form ───────────────────────────────────────────

export function TelegramForm({ onConnected, error, setError }: ChannelFormProps): VNode {
	const [accountId, setAccountId] = useState("");
	const [token, setToken] = useState("");
	const [dmPolicy, setDmPolicy] = useState("allowlist");
	const [allowlist, setAllowlist] = useState("");
	const [advancedConfig, setAdvancedConfig] = useState("");
	const [saving, setSaving] = useState(false);

	function onSubmit(e: Event): void {
		e.preventDefault();
		const v = validateChannelFields("telegram", accountId, token);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		const allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		const config: Record<string, unknown> = {
			token: token.trim(),
			dm_policy: dmPolicy,
			mention_mode: "mention",
			allowlist: allowlistEntries,
		};
		Object.assign(config, advancedPatch.value);
		(
			addChannel("telegram", accountId.trim(), config) as Promise<{
				ok?: boolean;
				error?: { message?: string; detail?: string };
			}>
		).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(accountId.trim(), "telegram");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect bot.");
			}
		});
	}

	return (
		<form onSubmit={onSubmit} className="flex flex-col gap-3">
			<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
				<span className="font-medium text-[var(--text-strong)]">How to create a Telegram bot</span>
				<span>
					1. Open{" "}
					<a href="https://t.me/BotFather" target="_blank" rel="noopener" className="text-[var(--accent)] underline">
						@BotFather
					</a>{" "}
					in Telegram
				</span>
				<span>2. Send /newbot and follow the prompts</span>
				<span>3. Copy the bot token and paste it below</span>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Bot username</label>
				<input
					type="text"
					className="provider-key-input w-full"
					value={accountId}
					onInput={(e) => setAccountId(targetValue(e))}
					placeholder="e.g. my_assistant_bot"
					autoComplete="off"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="telegram_bot_username"
					autoFocus
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Bot token (from @BotFather)</label>
				<input
					type="password"
					className="provider-key-input w-full"
					value={token}
					onInput={(e) => setToken(targetValue(e))}
					placeholder="123456:ABC-DEF..."
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="telegram_bot_token"
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
				<select
					className="provider-key-input w-full cursor-pointer"
					value={dmPolicy}
					onChange={(e) => setDmPolicy(targetValue(e))}
				>
					<option value="allowlist">Allowlist only (recommended)</option>
					<option value="open">Open (anyone)</option>
					<option value="disabled">Disabled</option>
				</select>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Your Telegram username(s)</label>
				<textarea
					className="provider-key-input w-full"
					rows={2}
					value={allowlist}
					onInput={(e) => setAllowlist(targetValue(e))}
					placeholder="your_username"
					style="resize:vertical;font-family:var(--font-body);"
				/>
				<div className="text-xs text-[var(--muted)] mt-1">
					One username per line, without the @ sign. These users can DM your bot.
				</div>
			</div>
			<AdvancedConfigPatchField value={advancedConfig} onInput={setAdvancedConfig} />
			{error && <ErrorPanel message={error} />}
			<button type="submit" className="provider-btn" disabled={saving}>
				{saving ? "Connecting\u2026" : "Connect Bot"}
			</button>
		</form>
	);
}

// ── Discord form ────────────────────────────────────────────

function discordInviteUrl(token: string): string {
	if (!token) return "";
	const parts = token.split(".");
	if (parts.length < 3) return "";
	try {
		const id = atob(parts[0]);
		if (!/^\d+$/.test(id)) return "";
		return `https://discord.com/oauth2/authorize?client_id=${id}&scope=bot&permissions=100352`;
	} catch {
		return "";
	}
}

export function DiscordForm({ onConnected, error, setError }: ChannelFormProps): VNode {
	const [accountId, setAccountId] = useState("");
	const [token, setToken] = useState("");
	const [dmPolicy, setDmPolicy] = useState("allowlist");
	const [allowlist, setAllowlist] = useState("");
	const [channelPatterns, setChannelPatterns] = useState("");
	const [advancedConfig, setAdvancedConfig] = useState("");
	const [saving, setSaving] = useState(false);

	function onSubmit(e: Event): void {
		e.preventDefault();
		const v = validateChannelFields("discord", accountId, token);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		const allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		const patternEntries = channelPatterns
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		const config: Record<string, unknown> = {
			token: token.trim(),
			dm_policy: dmPolicy,
			mention_mode: "mention",
			allowlist: allowlistEntries,
		};
		if (patternEntries.length > 0) config.channel_name_patterns = patternEntries;
		Object.assign(config, advancedPatch.value);
		(
			addChannel("discord", accountId.trim(), config) as Promise<{
				ok?: boolean;
				error?: { message?: string; detail?: string };
			}>
		).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(accountId.trim(), "discord");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect bot.");
			}
		});
	}

	const inviteUrl = discordInviteUrl(token);

	return (
		<form onSubmit={onSubmit} className="flex flex-col gap-3">
			<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
				<span className="font-medium text-[var(--text-strong)]">How to set up a Discord bot</span>
				<span>
					1. Go to the{" "}
					<a
						href="https://discord.com/developers/applications"
						target="_blank"
						rel="noopener"
						className="text-[var(--accent)] underline"
					>
						Discord Developer Portal
					</a>
				</span>
				<span>2. Create a new Application &rarr; Bot tab &rarr; copy the bot token</span>
				<span>
					3. Enable <strong>Message Content Intent</strong> under Privileged Gateway Intents
				</span>
				<span>4. Paste the token below &mdash; an invite link will be generated automatically</span>
				<span>5. You can also DM the bot directly without adding it to a server</span>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Account ID</label>
				<input
					type="text"
					className="provider-key-input w-full"
					value={accountId}
					onInput={(e) => setAccountId(targetValue(e))}
					placeholder="e.g. my_discord_bot"
					autoComplete="off"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="discord_account_id"
					autoFocus
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Bot token</label>
				<input
					type="password"
					className="provider-key-input w-full"
					value={token}
					onInput={(e) => setToken(targetValue(e))}
					placeholder="Bot token from Developer Portal"
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="discord_bot_token"
				/>
			</div>
			{inviteUrl && (
				<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2.5 text-xs flex flex-col gap-1">
					<span className="font-medium text-[var(--text-strong)]">Invite bot to a server</span>
					<span className="text-[var(--muted)]">
						Open this link to add the bot (Send Messages, Attach Files, Read Message History):
					</span>
					<a href={inviteUrl} target="_blank" rel="noopener" className="text-[var(--accent)] underline break-all">
						{inviteUrl}
					</a>
				</div>
			)}
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
				<select
					className="provider-key-input w-full cursor-pointer"
					value={dmPolicy}
					onChange={(e) => setDmPolicy(targetValue(e))}
				>
					<option value="allowlist">Allowlist only (recommended)</option>
					<option value="open">Open (anyone)</option>
					<option value="disabled">Disabled</option>
				</select>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Allowed Discord username(s)</label>
				<textarea
					className="provider-key-input w-full"
					rows={2}
					value={allowlist}
					onInput={(e) => setAllowlist(targetValue(e))}
					placeholder="your_username"
					style="resize:vertical;font-family:var(--font-body);"
				/>
				<div className="text-xs text-[var(--muted)] mt-1">One username per line. These users can DM your bot.</div>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Channel Name Patterns (optional)</label>
				<textarea
					className="provider-key-input w-full"
					rows={2}
					value={channelPatterns}
					onInput={(e) => setChannelPatterns(targetValue(e))}
					placeholder="ticket-*"
					style="resize:vertical;font-family:var(--font-body);"
				/>
				<div className="text-xs text-[var(--muted)] mt-1">
					One glob pattern per line. When set, the bot only responds in matching guild channels (no @mention needed).
					Supports * wildcards. E.g., ticket-*, support-*.
				</div>
			</div>
			<AdvancedConfigPatchField value={advancedConfig} onInput={setAdvancedConfig} />
			{error && <ErrorPanel message={error} />}
			<button type="submit" className="provider-btn" disabled={saving}>
				{saving ? "Connecting\u2026" : "Connect Bot"}
			</button>
		</form>
	);
}

// ── Nostr form ──────────────────────────────────────────────

export function NostrForm({ onConnected, error, setError }: ChannelFormProps): VNode {
	const [accountId, setAccountId] = useState("");
	const [secretKey, setSecretKey] = useState("");
	const [relays, setRelays] = useState("wss://relay.damus.io, wss://relay.nostr.band, wss://nos.lol");
	const [dmPolicy, setDmPolicy] = useState("allowlist");
	const [allowlist, setAllowlist] = useState("");
	const [advancedConfig, setAdvancedConfig] = useState("");
	const [saving, setSaving] = useState(false);

	function onSubmit(e: Event): void {
		e.preventDefault();
		if (!accountId.trim()) {
			setError("Account ID is required.");
			return;
		}
		if (!secretKey.trim()) {
			setError("Secret key is required.");
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		const relayList = relays
			.split(",")
			.map((r) => r.trim())
			.filter(Boolean);
		const allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		const config: Record<string, unknown> = {
			secret_key: secretKey.trim(),
			relays: relayList,
			dm_policy: dmPolicy,
			allowed_pubkeys: allowlistEntries,
		};
		Object.assign(config, advancedPatch.value);
		(
			addChannel("nostr", accountId.trim(), config) as Promise<{
				ok?: boolean;
				error?: { message?: string; detail?: string };
			}>
		).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(accountId.trim(), "nostr");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect channel.");
			}
		});
	}

	return (
		<form onSubmit={onSubmit} className="flex flex-col gap-3">
			<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
				<span className="font-medium text-[var(--text-strong)]">How to set up Nostr DMs</span>
				<span>1. Generate or use an existing Nostr secret key (nsec1... or hex)</span>
				<span>2. Configure relay URLs (defaults are provided)</span>
				<span>3. Add allowed public keys (npub1... or hex) to the allowlist</span>
				<span>4. Send a DM to the bot's public key from any Nostr client</span>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Account ID</label>
				<input
					type="text"
					className="provider-key-input w-full"
					value={accountId}
					onInput={(e) => setAccountId(targetValue(e))}
					placeholder="e.g. my-nostr-bot"
					autoComplete="off"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="nostr_account_id"
					autoFocus
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Secret Key</label>
				<input
					type="password"
					className="provider-key-input w-full"
					value={secretKey}
					onInput={(e) => setSecretKey(targetValue(e))}
					placeholder="nsec1... or 64-char hex"
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="nostr_secret_key"
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Relays (comma-separated)</label>
				<input
					type="text"
					className="provider-key-input w-full"
					value={relays}
					onInput={(e) => setRelays(targetValue(e))}
					placeholder="wss://relay.damus.io, wss://nos.lol"
					name="nostr_relays"
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
				<select className="channel-select w-full" value={dmPolicy} onChange={(e) => setDmPolicy(targetValue(e))}>
					<option value="allowlist">Allowlist only</option>
					<option value="open">Open (anyone)</option>
					<option value="disabled">Disabled</option>
				</select>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">
					Allowed Public Keys (one per line, npub1 or hex)
				</label>
				<textarea
					className="provider-key-input w-full"
					rows={3}
					value={allowlist}
					onInput={(e) => setAllowlist(targetValue(e))}
					placeholder={"npub1abc123...\nnpub1def456..."}
					name="nostr_allowed_pubkeys"
				/>
			</div>
			<AdvancedConfigPatchField value={advancedConfig} onInput={setAdvancedConfig} />
			{error && <div className="text-xs text-[var(--error)]">{error}</div>}
			<button type="submit" className="provider-btn self-start" disabled={saving}>
				{saving ? "Connecting\u2026" : "Connect Nostr"}
			</button>
		</form>
	);
}

// ── Signal form ──────────────────────────────────────────────

export function SignalForm({ onConnected, error, setError }: ChannelFormProps): VNode {
	const [account, setAccount] = useState("");
	const [httpUrl, setHttpUrl] = useState("http://127.0.0.1:8080");
	const [dmPolicy, setDmPolicy] = useState("allowlist");
	const [groupPolicy, setGroupPolicy] = useState("disabled");
	const [allowlist, setAllowlist] = useState("");
	const [groupAllowlist, setGroupAllowlist] = useState("");
	const [advancedConfig, setAdvancedConfig] = useState("");
	const [saving, setSaving] = useState(false);

	function splitLines(value: string): string[] {
		return value
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
	}

	function onSubmit(e: Event): void {
		e.preventDefault();
		if (!account.trim()) {
			setError("Signal account (phone number) is required.");
			return;
		}
		if (!httpUrl.trim()) {
			setError("signal-cli daemon URL is required.");
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		const accountId = deriveSignalAccountId(account);
		const config: Record<string, unknown> = {
			http_url: httpUrl.trim(),
			dm_policy: dmPolicy,
			allowlist: splitLines(allowlist),
			group_policy: groupPolicy,
			group_allowlist: splitLines(groupAllowlist),
			mention_mode: "mention",
			account: account.trim(),
		};
		Object.assign(config, advancedPatch.value);
		(
			addChannel("signal", accountId, config) as Promise<{
				ok?: boolean;
				error?: { message?: string; detail?: string };
			}>
		).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(accountId, "signal");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect Signal.");
			}
		});
	}

	return (
		<form onSubmit={onSubmit} className="flex flex-col gap-3">
			<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
				<span className="font-medium text-[var(--text-strong)]">Requires signal-cli</span>
				{/* biome-ignore lint: single-line keeps whitespace intact */}
				<span>
					Signal integration requires a running{" "}
					<a
						href="https://github.com/AsamK/signal-cli"
						target="_blank"
						rel="noopener noreferrer"
						className="underline text-[var(--text-strong)]"
					>
						signal-cli
					</a>{" "}
					daemon with JSON-RPC HTTP enabled. Install it, register or link your Signal account, then start the daemon:
				</span>
				<code className="text-[10px] bg-[var(--surface1)] px-1.5 py-0.5 rounded mt-0.5">
					signal-cli daemon --http localhost:8080
				</code>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Signal Account (phone number)</label>
				<input
					type="text"
					className="provider-key-input w-full"
					value={account}
					onInput={(e) => setAccount(targetValue(e))}
					placeholder="+15551234567"
					autoComplete="off"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="signal_account"
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">signal-cli Daemon URL</label>
				<input
					type="url"
					className="provider-key-input w-full"
					value={httpUrl}
					onInput={(e) => setHttpUrl(targetValue(e))}
					placeholder="http://127.0.0.1:8080"
					name="signal_http_url"
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
				<select className="channel-select w-full" value={dmPolicy} onChange={(e) => setDmPolicy(targetValue(e))}>
					<option value="allowlist">Allowlist only</option>
					<option value="open">Open (anyone)</option>
					<option value="disabled">Disabled</option>
				</select>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Group Policy</label>
				<select className="channel-select w-full" value={groupPolicy} onChange={(e) => setGroupPolicy(targetValue(e))}>
					<option value="disabled">Disabled</option>
					<option value="allowlist">Allowlist only</option>
					<option value="open">Open (any group)</option>
				</select>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">DM Allowlist</label>
				<textarea
					className="provider-key-input w-full"
					rows={2}
					value={allowlist}
					onInput={(e) => setAllowlist(targetValue(e))}
					placeholder={"+15551234567\n550e8400-e29b-41d4-a716-446655440000"}
					name="signal_allowlist"
				/>
			</div>
			<div>
				<label className="text-xs text-[var(--muted)] mb-1 block">Group Allowlist</label>
				<textarea
					className="provider-key-input w-full"
					rows={2}
					value={groupAllowlist}
					onInput={(e) => setGroupAllowlist(targetValue(e))}
					placeholder="base64-encoded Signal group ID"
					name="signal_group_allowlist"
				/>
			</div>
			<AdvancedConfigPatchField value={advancedConfig} onInput={setAdvancedConfig} />
			{error && <div className="text-xs text-[var(--error)]">{error}</div>}
			<button type="submit" className="provider-btn self-start" disabled={saving}>
				{saving ? "Connecting\u2026" : "Connect Signal"}
			</button>
		</form>
	);
}
