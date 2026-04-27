// ── User Profile section ─────────────────────────────────────
//
// User-level settings: your name, UI language, version display.
// Agent identity (name, emoji, theme, soul) is now managed on the
// Agents page via inline editing.

import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { SectionHeading, StatusMessage, SubHeading } from "../../components/forms";
import * as gon from "../../gon";
import { refresh as refreshGon } from "../../gon";
import { setLocale } from "../../i18n";
import { updateIdentity } from "../../identity-utils";
import { targetValue } from "../../typed-events";
import type { IdentityData, RpcResponse } from "./_shared";
import { identity, loading, rerender } from "./_shared";

export function IdentitySection(): VNode {
	const id = identity.value;
	const storedLocale = localStorage.getItem("moltis-locale");

	const [userName, setUserName] = useState(id?.user_name || "");
	const [uiLanguage, setUiLanguage] = useState(storedLocale || "auto");
	const [userNameSaving, setUserNameSaving] = useState(false);
	const [languageSaving, setLanguageSaving] = useState(false);
	const [saved, setSaved] = useState(false);
	const [languageSaved, setLanguageSaved] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [languageError, setLanguageError] = useState<string | null>(null);

	useEffect(() => {
		if (!id) return;
		setUserName(id.user_name || "");
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

	function onUserNameBlur(e: Event): void {
		const trimmed = targetValue(e).trim();
		const currentValue = (identity.value?.user_name || "").trim();
		if (trimmed === currentValue) return;
		if (!trimmed) {
			setError("Your name is required.");
			return;
		}
		setError(null);
		setSaved(false);
		setUserNameSaving(true);
		updateIdentity({ user_name: trimmed }, { agentId: "main" }).then((res: RpcResponse) => {
			setUserNameSaving(false);
			if (res?.ok) {
				const payload = res.payload as IdentityData;
				identity.value = payload;
				gon.set("identity", payload as import("../../types/gon").ResolvedIdentity);
				refreshGon();
				setUserName(payload?.user_name || "");
				flashSaved();
			} else {
				setError(res?.error?.message || "Failed to save");
			}
			rerender();
		});
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
			<SectionHeading title="User Profile" />
			<div style={{ maxWidth: "600px", display: "flex", flexDirection: "column", gap: "16px" }}>
				{/* User name */}
				<div>
					<SubHeading title="Your Name" />
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Saved to your user profile. Depending on memory settings, Moltis may also mirror it to <code>USER.md</code>.
					</p>
					<div className="flex items-center gap-2">
						<input
							type="text"
							className="provider-key-input"
							style={{ width: "100%", maxWidth: "280px" }}
							value={userName}
							onInput={(e: Event) => setUserName(targetValue(e))}
							onBlur={onUserNameBlur}
							placeholder="e.g. Alice"
						/>
						{userNameSaving && <span className="text-xs text-[var(--muted)]">Saving{"\u2026"}</span>}
						<StatusMessage error={error} success={saved ? "Saved" : null} />
					</div>
				</div>

				{/* Language */}
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
			</div>
			{gon.get("version") ? (
				<p className="text-xs text-[var(--muted)]" style={{ marginTop: "auto", paddingTop: "16px" }}>
					v{gon.get("version")}
				</p>
			) : null}
		</div>
	);
}
