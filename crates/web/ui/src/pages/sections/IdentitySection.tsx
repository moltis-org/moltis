// ── Identity section (editable form) ─────────────────────────

import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { SectionHeading, StatusMessage, SubHeading } from "../../components/forms";
import { EmojiPicker } from "../../emoji-picker";
import * as gon from "../../gon";
import { refresh as refreshGon } from "../../gon";
import { setLocale } from "../../i18n";
import { updateIdentity, validateIdentityFields } from "../../identity-utils";
import { targetValue } from "../../typed-events";
import type { IdentityData, RpcResponse } from "./_shared";
import { identity, isSafariBrowser, loading, rerender } from "./_shared";

// ── Soul defaults ────────────────────────────────────────────

export const DEFAULT_SOUL =
	"Be genuinely helpful, not performatively helpful. Skip the filler words \u2014 just help.\n" +
	"Have opinions. You're allowed to disagree, prefer things, find stuff amusing or boring.\n" +
	"Be resourceful before asking. Try to figure it out first \u2014 read the context, search for it \u2014 then ask if you're stuck.\n" +
	"Earn trust through competence. Be careful with external actions. Be bold with internal ones.\n" +
	"Remember you're a guest. You have access to someone's life. Treat it with respect.\n" +
	"Private things stay private. When in doubt, ask before acting externally.\n" +
	"Be concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just good.";

export function IdentitySection(): VNode {
	const id = identity.value;
	const isNew = !(id && (id.name || id.user_name));
	const storedLocale = localStorage.getItem("moltis-locale");

	const [name, setName] = useState(id?.name || "");
	const [emoji, setEmoji] = useState(id?.emoji || "");
	const [theme, setTheme] = useState(id?.theme || "");
	const [userName, setUserName] = useState(id?.user_name || "");
	const [soul, setSoul] = useState(id?.soul || "");
	const [uiLanguage, setUiLanguage] = useState(storedLocale || "auto");
	const [saving, setSaving] = useState(false);
	const [emojiSaving, setEmojiSaving] = useState(false);
	const [nameSaving, setNameSaving] = useState(false);
	const [userNameSaving, setUserNameSaving] = useState(false);
	const [languageSaving, setLanguageSaving] = useState(false);
	const [saved, setSaved] = useState(false);
	const [languageSaved, setLanguageSaved] = useState(false);
	const [showFaviconReloadHint, setShowFaviconReloadHint] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [languageError, setLanguageError] = useState<string | null>(null);

	// Sync state when identity loads asynchronously
	useEffect(() => {
		if (!id) return;
		setName(id.name || "");
		setEmoji(id.emoji || "");
		setTheme(id.theme || "");
		setUserName(id.user_name || "");
		setSoul(id.soul || "");
	}, [id]);

	const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	function flashSaved(): void {
		if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
		setSaved(true);
		savedTimerRef.current = setTimeout(() => {
			savedTimerRef.current = null;
			setSaved(false);
			rerender();
		}, 2000);
	}

	if (loading.value) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	function onSave(e: Event): void {
		e.preventDefault();
		const v = validateIdentityFields(name, userName);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		setError(null);
		setSaving(true);
		setSaved(false);

		updateIdentity(
			{
				name: name.trim(),
				emoji: emoji.trim() || "",
				theme: theme.trim() || "",
				soul: soul.trim() || undefined,
				user_name: userName.trim(),
			},
			{ agentId: "main" },
		).then((res: RpcResponse) => {
			setSaving(false);
			if (res?.ok) {
				const payload = res.payload as IdentityData;
				identity.value = payload;
				gon.set("identity", payload as import("../../types/gon").ResolvedIdentity);
				refreshGon();
				const emojiChanged = (emoji.trim() || "") !== (id?.emoji || "").trim();
				setShowFaviconReloadHint(emojiChanged && isSafariBrowser());
				flashSaved();
			} else {
				setError(res?.error?.message || "Failed to save");
			}
			rerender();
		});
	}

	function onEmojiSelect(nextEmoji: string): void {
		setEmoji(nextEmoji);
		setError(null);
		setSaved(false);
		setEmojiSaving(true);
		updateIdentity({ emoji: nextEmoji.trim() || "" }, { agentId: "main" }).then((res: RpcResponse) => {
			setEmojiSaving(false);
			if (res?.ok) {
				const payload = res.payload as IdentityData;
				identity.value = payload;
				setEmoji(payload?.emoji || "");
				gon.set("identity", payload as import("../../types/gon").ResolvedIdentity);
				refreshGon();
				const emojiChanged = (nextEmoji.trim() || "") !== (id?.emoji || "").trim();
				setShowFaviconReloadHint(emojiChanged && isSafariBrowser());
				flashSaved();
			} else {
				setError(res?.error?.message || "Failed to save emoji");
			}
			rerender();
		});
	}

	function autoSaveNameField(field: string, value: string): void {
		if (saving || emojiSaving || nameSaving || userNameSaving) return;
		const trimmed = value.trim();
		const currentValue = ((identity.value?.[field] as string) || "").trim();
		if (trimmed === currentValue) return;

		if (!trimmed) {
			setError(field === "name" ? "Agent name is required." : "Your name is required.");
			return;
		}

		setError(null);
		setSaved(false);
		if (field === "name") {
			setNameSaving(true);
		} else {
			setUserNameSaving(true);
		}

		const payload: Record<string, string> = {};
		payload[field] = trimmed;
		updateIdentity(payload, { agentId: "main" }).then((res: RpcResponse) => {
			if (field === "name") {
				setNameSaving(false);
			} else {
				setUserNameSaving(false);
			}

			if (res?.ok) {
				const resPayload = res.payload as IdentityData;
				identity.value = resPayload;
				gon.set("identity", resPayload as import("../../types/gon").ResolvedIdentity);
				refreshGon();
				setName(resPayload?.name || "");
				setUserName(resPayload?.user_name || "");
				flashSaved();
			} else {
				setError(res?.error?.message || "Failed to save");
			}
			rerender();
		});
	}

	function onNameBlur(e: Event): void {
		autoSaveNameField("name", targetValue(e));
	}

	function onUserNameBlur(e: Event): void {
		autoSaveNameField("user_name", targetValue(e));
	}

	function onResetSoul(): void {
		setSoul("");
		rerender();
	}

	function onReloadForFavicon(): void {
		window.location.reload();
	}

	function onApplyLanguage(): void {
		setLanguageSaving(true);
		setLanguageSaved(false);
		setLanguageError(null);

		const nextLanguage = uiLanguage === "auto" ? navigator.language || "en" : uiLanguage;
		setLocale(nextLanguage)
			.then(() => {
				if (uiLanguage === "auto") {
					localStorage.removeItem("moltis-locale");
				}
				setLanguageSaving(false);
				setLanguageSaved(true);
				setTimeout(() => {
					setLanguageSaved(false);
					rerender();
				}, 2000);
				rerender();
			})
			.catch((err: Error) => {
				setLanguageSaving(false);
				setLanguageError(err?.message || "Failed to update language");
				rerender();
			});
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="Identity" />
			{isNew ? (
				<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
					Welcome! Set up your agent's identity to get started.
				</p>
			) : null}
			<form onSubmit={onSave} style={{ maxWidth: "600px", display: "flex", flexDirection: "column", gap: "16px" }}>
				{/* Agent section */}
				<div>
					<SubHeading title="Agent" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Saved to <code>IDENTITY.md</code> in your workspace root.
					</p>
					<div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "8px 16px" }}>
						<div>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								Name *
							</div>
							<input
								type="text"
								className="provider-key-input"
								style={{ width: "100%" }}
								value={name}
								onInput={(e: Event) => setName(targetValue(e))}
								onBlur={onNameBlur}
								placeholder="e.g. Rex"
							/>
						</div>
						<div>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								Emoji
							</div>
							<EmojiPicker value={emoji} onChange={setEmoji} onSelect={onEmojiSelect} />
						</div>
						<div style={{ gridColumn: "1/-1" }}>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								Theme
							</div>
							<input
								type="text"
								className="provider-key-input"
								style={{ width: "100%" }}
								value={theme}
								onInput={(e: Event) => setTheme(targetValue(e))}
								placeholder="e.g. wise owl, chill fox"
							/>
						</div>
					</div>
					{showFaviconReloadHint ? (
						<div className="mt-3 rounded border border-[var(--border)] bg-[var(--surface2)] p-2 text-xs text-[var(--muted)]">
							favicon updates requires reload and may be cached for minutes,{" "}
							<button
								type="button"
								className="cursor-pointer bg-transparent p-0 text-xs text-[var(--text)] underline"
								onClick={onReloadForFavicon}
							>
								requires reload
							</button>
							.
						</div>
					) : null}
				</div>

				{/* User section */}
				<div>
					<SubHeading title="User" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Saved to your user profile. Depending on memory settings, Moltis may also mirror it to <code>USER.md</code>.
					</p>
					<div>
						<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
							Your name *
						</div>
						<input
							type="text"
							className="provider-key-input"
							style={{ width: "100%", maxWidth: "280px" }}
							value={userName}
							onInput={(e: Event) => setUserName(targetValue(e))}
							onBlur={onUserNameBlur}
							placeholder="e.g. Alice"
						/>
					</div>
				</div>

				{/* Language section */}
				<div>
					<SubHeading title="Language" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Choose the UI language for this browser.
					</p>
					<div style={{ display: "flex", alignItems: "center", gap: "8px", flexWrap: "wrap" }}>
						<label htmlFor="identityLanguageSelect" className="text-xs text-[var(--muted)]">
							UI language
						</label>
						<select
							id="identityLanguageSelect"
							className="provider-key-input"
							style={{ maxWidth: "220px" }}
							value={uiLanguage}
							onChange={(e: Event) => {
								setUiLanguage(targetValue(e));
								setLanguageSaved(false);
								setLanguageError(null);
								rerender();
							}}
						>
							<option value="auto">Browser default</option>
							<option value="en">English</option>
							<option value="fr">French</option>
							<option value="zh">{"\u7B80\u4F53\u4E2D\u6587"}</option>
						</select>
						<button
							type="button"
							id="identityLanguageApplyBtn"
							className="provider-btn provider-btn-secondary"
							disabled={languageSaving}
							onClick={onApplyLanguage}
						>
							{languageSaving ? "Applying..." : "Apply language"}
						</button>
						<StatusMessage error={languageError} success={languageSaved ? "Language updated" : null} />
					</div>
				</div>

				{/* Soul section */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "4px" }}>
						Soul
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Personality and tone injected into every conversation. Saved to <code>SOUL.md</code> in your workspace root.
						Leave empty for the default.
					</p>
					<textarea
						className="provider-key-input"
						rows={8}
						style={{ width: "100%", minHeight: "8rem", resize: "vertical", fontSize: ".8rem", lineHeight: 1.5 }}
						placeholder={DEFAULT_SOUL}
						value={soul}
						onInput={(e: Event) => setSoul(targetValue(e))}
					/>
					{soul ? (
						<button
							type="button"
							className="provider-btn"
							style={{ marginTop: "6px", fontSize: ".75rem" }}
							onClick={onResetSoul}
						>
							Reset to default
						</button>
					) : null}
				</div>

				<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
					<button
						type="submit"
						className="provider-btn"
						disabled={saving || emojiSaving || nameSaving || userNameSaving}
					>
						{saving || emojiSaving || nameSaving || userNameSaving ? "Saving\u2026" : "Save"}
					</button>
					<StatusMessage error={error} success={saved ? "Saved" : null} />
				</div>
			</form>
			{gon.get("version") ? (
				<p className="text-xs text-[var(--muted)]" style={{ marginTop: "auto", paddingTop: "16px" }}>
					v{gon.get("version")}
				</p>
			) : null}
		</div>
	);
}
