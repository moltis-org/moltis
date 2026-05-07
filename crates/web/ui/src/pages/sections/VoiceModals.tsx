// ── Voice modals — extracted from VoiceSection ──────────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import * as gon from "../../gon";
import { sendRpc } from "../../helpers";
import { fetchPhrase } from "../../tts-phrases";
import { targetValue } from "../../typed-events";
import { Modal } from "../../ui";
import {
	createVoicePersona,
	decodeBase64Safe,
	saveVoiceKey,
	saveVoiceSettings,
	testTtsWithPersona,
	updateVoicePersona,
	type VoicePersonaPrompt,
	type VoicePersonaProviderBinding,
	type VoicePersonaResponse,
} from "../../voice-utils";
import type { RpcResponse } from "./_shared";
import { rerender } from "./_shared";
import { cloneHidden } from "./RemoteAccessSection";

// ── Shared signals ──────────────────────────────────────────

export const voiceShowAddModal = signal(false);
export const voiceSelectedProvider = signal<string | null>(null);
export const voiceSelectedProviderData = signal<VoiceProviderData | null>(null);

// ── Shared interfaces ───────────────────────────────────────

export interface VoiceProviderData {
	id: string;
	name: string;
	description?: string;
	type?: string;
	category?: string;
	available?: boolean;
	enabled?: boolean;
	preferred?: boolean;
	keySource?: string;
	settingsSummary?: string;
	binaryPath?: string;
	statusMessage?: string;
	keyPlaceholder?: string;
	keyUrl?: string;
	keyUrlLabel?: string;
	hint?: string;
	capabilities?: { baseUrl?: boolean };
	settings?: { baseUrl?: string; voiceId?: string; voice?: string; model?: string; languageCode?: string };
}

export interface VoiceTesting {
	id: string;
	type: string;
	phase: string;
}

export interface VoiceTestResult {
	text?: string | null;
	success?: boolean;
	error?: string | null;
}

export interface VoxtralRequirements {
	os?: string;
	arch?: string;
	compatible?: boolean;
	reasons?: string[];
	python?: { available?: boolean; version?: string };
	cuda?: { available?: boolean; gpu_name?: string; memory_mb?: number };
}

// ── PersonaEditModal ────────────────────────────────────────

interface PersonaEditModalProps {
	editingId: string;
	existingPersona: VoicePersonaResponse | null;
	onClose: () => void;
	onSaved: () => void;
}

export function PersonaEditModal({ editingId, existingPersona, onClose, onSaved }: PersonaEditModalProps): VNode {
	const isNew = editingId === "__new__";
	const [id, setId] = useState(existingPersona?.persona.id ?? "");
	const [label, setLabel] = useState(existingPersona?.persona.label ?? "");
	const [description, setDescription] = useState(existingPersona?.persona.description ?? "");
	const [profile, setProfile] = useState(existingPersona?.persona.prompt.profile ?? "");
	const [style, setStyle] = useState(existingPersona?.persona.prompt.style ?? "");
	const [accent, setAccent] = useState(existingPersona?.persona.prompt.accent ?? "");
	const [pacing, setPacing] = useState(existingPersona?.persona.prompt.pacing ?? "");
	const [saving, setSaving] = useState(false);
	const [testing, setTesting] = useState(false);
	const [error, setError] = useState<string | null>(null);

	// Provider bindings state
	const existingBindings = existingPersona?.persona.provider_bindings ?? [];
	const findBinding = (prov: string): VoicePersonaProviderBinding | undefined =>
		existingBindings.find((b) => b.provider === prov);
	const [openaiVoice, setOpenaiVoice] = useState(findBinding("openai")?.voice_id ?? "");
	const [openaiModel, setOpenaiModel] = useState(findBinding("openai")?.model ?? "gpt-4o-mini-tts");
	const [elevenVoice, setElevenVoice] = useState(findBinding("elevenlabs")?.voice_id ?? "");

	function buildBindings(): VoicePersonaProviderBinding[] {
		const bindings: VoicePersonaProviderBinding[] = [];
		if (openaiVoice || openaiModel) {
			bindings.push({
				provider: "openai",
				voice_id: openaiVoice || undefined,
				model: openaiModel || undefined,
			});
		}
		if (elevenVoice) {
			bindings.push({ provider: "elevenlabs", voice_id: elevenVoice });
		}
		return bindings;
	}

	async function savePersona(): Promise<boolean> {
		setSaving(true);
		setError(null);
		try {
			const prompt: VoicePersonaPrompt = {};
			if (profile) prompt.profile = profile;
			if (style) prompt.style = style;
			if (accent) prompt.accent = accent;
			if (pacing) prompt.pacing = pacing;
			const providerBindings = buildBindings();

			if (isNew) {
				if (!(id && label)) {
					setError("ID and Label are required.");
					setSaving(false);
					return false;
				}
				await createVoicePersona({
					id,
					label,
					description: description || undefined,
					prompt,
					providerBindings,
				});
			} else {
				await updateVoicePersona(editingId, {
					label: label || undefined,
					description: description || undefined,
					prompt,
					providerBindings,
				});
			}
			return true;
		} catch (err: unknown) {
			setError(err instanceof Error ? err.message : String(err));
			return false;
		} finally {
			setSaving(false);
		}
	}

	async function handleSave(): Promise<void> {
		if (await savePersona()) {
			onSaved();
		}
	}

	async function handleTest(): Promise<void> {
		setTesting(true);
		try {
			const personaIdToTest = isNew ? undefined : editingId;
			const identity = gon.get("identity") as { user_name?: string; name?: string } | undefined;
			const user = identity?.user_name || "friend";
			const bot = label || identity?.name || "Moltis";
			const text = await fetchPhrase("settings", user, bot);

			let res: RpcResponse;
			if (personaIdToTest) {
				// Save first so latest changes are used, then test (without closing the modal).
				await savePersona();
				res = (await testTtsWithPersona(text, personaIdToTest)) as RpcResponse;
			} else {
				// New persona not yet saved — test with raw tts.convert and manual instructions.
				const prompt: VoicePersonaPrompt = {};
				if (profile) prompt.profile = profile;
				if (style) prompt.style = style;
				if (accent) prompt.accent = accent;
				if (pacing) prompt.pacing = pacing;
				const instructions = [
					`Persona: ${label || "Test"}`,
					prompt.profile ? `Profile: ${prompt.profile}` : "",
					prompt.style ? `Style: ${prompt.style}` : "",
					prompt.accent ? `Accent: ${prompt.accent}` : "",
					prompt.pacing ? `Pacing: ${prompt.pacing}` : "",
				]
					.filter(Boolean)
					.join("\n");
				const params: Record<string, unknown> = { text };
				if (instructions) params.instructions = instructions;
				if (openaiVoice) params.voiceId = openaiVoice;
				if (openaiModel) params.model = openaiModel;
				res = (await sendRpc("tts.convert", params)) as RpcResponse;
			}

			if (res?.ok) {
				const payload = res.payload as { audio?: string; mimeType?: string };
				if (payload?.audio) {
					const bytes = decodeBase64Safe(payload.audio);
					const blob = new Blob([bytes as BlobPart], { type: payload.mimeType || "audio/mpeg" });
					const url = URL.createObjectURL(blob);
					const audio = new Audio(url);
					audio.onended = () => URL.revokeObjectURL(url);
					audio.play().catch((e: Error) => console.error("[TTS]", e));
				}
			}
		} catch (_e) {
			/* ignore */
		} finally {
			setTesting(false);
		}
	}

	return (
		<Modal show onClose={onClose} title={isNew ? "New Voice Persona" : `Edit ${label}`}>
			<div
				className="channel-form"
				style={{
					display: "flex",
					flexDirection: "column",
					gap: "12px",
					padding: "16px",
					maxHeight: "70vh",
					overflowY: "auto",
				}}
			>
				{isNew ? (
					<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
						ID (lowercase, no spaces)
						<input
							className="provider-key-input w-full"
							placeholder="alfred"
							value={id}
							onInput={(e) => setId(targetValue(e))}
						/>
					</label>
				) : null}
				<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
					Display Name
					<input
						className="provider-key-input w-full"
						placeholder="Alfred the Butler"
						value={label}
						onInput={(e) => setLabel(targetValue(e))}
					/>
				</label>
				<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
					Description
					<input
						className="provider-key-input w-full"
						placeholder="A wise British butler with dry wit"
						value={description}
						onInput={(e) => setDescription(targetValue(e))}
					/>
				</label>

				<hr style={{ border: "none", borderTop: "1px solid var(--border)", margin: "4px 0" }} />
				<p className="text-xs text-[var(--muted)]" style={{ margin: 0 }}>
					Voice direction — controls tone on providers that support instructions (OpenAI gpt-4o-mini-tts, Gemini TTS).
				</p>
				<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
					Character Profile
					<input
						className="provider-key-input w-full"
						placeholder="A wise British butler, dry wit, formal"
						value={profile}
						onInput={(e) => setProfile(targetValue(e))}
					/>
				</label>
				<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
					Delivery Style
					<input
						className="provider-key-input w-full"
						placeholder="Measured, deliberate, slightly amused"
						value={style}
						onInput={(e) => setStyle(targetValue(e))}
					/>
				</label>
				<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
					Accent
					<input
						className="provider-key-input w-full"
						placeholder="Received Pronunciation"
						value={accent}
						onInput={(e) => setAccent(targetValue(e))}
					/>
				</label>
				<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
					Pacing
					<input
						className="provider-key-input w-full"
						placeholder="Unhurried, with dramatic pauses"
						value={pacing}
						onInput={(e) => setPacing(targetValue(e))}
					/>
				</label>

				<hr style={{ border: "none", borderTop: "1px solid var(--border)", margin: "4px 0" }} />
				<p className="text-xs text-[var(--muted)]" style={{ margin: 0 }}>
					Provider bindings — voice and model overrides per TTS provider.
				</p>
				<div
					className="flex flex-col gap-2 p-2 rounded border border-[var(--border)]"
					style={{ background: "var(--surface)" }}
				>
					<span className="text-xs font-medium text-[var(--text-strong)]">OpenAI TTS</span>
					<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
						Voice (alloy, echo, fable, onyx, nova, shimmer, coral, cedar, ...)
						<input
							className="provider-key-input w-full"
							placeholder="alloy"
							value={openaiVoice}
							onInput={(e) => setOpenaiVoice(targetValue(e))}
						/>
					</label>
					<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
						Model
						<input
							className="provider-key-input w-full"
							placeholder="gpt-4o-mini-tts"
							value={openaiModel}
							onInput={(e) => setOpenaiModel(targetValue(e))}
						/>
					</label>
				</div>
				<div
					className="flex flex-col gap-2 p-2 rounded border border-[var(--border)]"
					style={{ background: "var(--surface)" }}
				>
					<span className="text-xs font-medium text-[var(--text-strong)]">ElevenLabs</span>
					<label className="text-xs text-[var(--muted)] flex flex-col gap-1">
						Voice ID
						<input
							className="provider-key-input w-full"
							placeholder="21m00Tcm4TlvDq8ikWAM"
							value={elevenVoice}
							onInput={(e) => setElevenVoice(targetValue(e))}
						/>
					</label>
				</div>

				{error ? <div className="text-xs text-[var(--error)]">{error}</div> : null}
				<div className="flex gap-2 justify-end" style={{ marginTop: "8px" }}>
					<button type="button" className="provider-btn provider-btn-secondary" disabled={testing} onClick={handleTest}>
						{testing ? "Testing..." : "Test Voice"}
					</button>
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						Cancel
					</button>
					<button type="button" className="provider-btn" disabled={saving} onClick={handleSave}>
						{saving ? "Saving..." : isNew ? "Create" : "Save"}
					</button>
				</div>
			</div>
		</Modal>
	);
}

// ── LocalProviderInstructions (used only by AddVoiceProviderModal) ──

interface LocalProviderInstructionsProps {
	providerId: string;
	voxtralReqs: VoxtralRequirements | null;
}

function LocalProviderInstructions({ providerId, voxtralReqs }: LocalProviderInstructionsProps): VNode {
	const ref = useRef<HTMLDivElement>(null);

	useEffect(() => {
		const container = ref.current;
		if (!container) return;
		while (container.firstChild) container.removeChild(container.firstChild);

		const templateId: Record<string, string> = {
			"whisper-cli": "voice-whisper-cli-instructions",
			"whisper-local": "voice-whisper-local-instructions",
			"sherpa-onnx": "voice-sherpa-onnx-instructions",
			piper: "voice-piper-instructions",
			coqui: "voice-coqui-instructions",
			"voxtral-local": "voice-voxtral-instructions",
		};

		const tplId = templateId[providerId];
		if (!tplId) return;

		const el = cloneHidden(tplId);
		if (!el) return;

		if (providerId === "voxtral-local" && el.querySelector("[data-voxtral-requirements]")) {
			const reqsContainer = el.querySelector("[data-voxtral-requirements]") as HTMLElement;
			if (voxtralReqs) {
				let detected = `${voxtralReqs.os}/${voxtralReqs.arch}`;
				if (voxtralReqs.python?.available) detected += `, Python ${voxtralReqs.python.version}`;
				else detected += ", no Python";
				if (voxtralReqs.cuda?.available) {
					detected += `, ${voxtralReqs.cuda.gpu_name || "NVIDIA GPU"} (${Math.round((voxtralReqs.cuda.memory_mb || 0) / 1024)}GB)`;
				} else detected += ", no CUDA GPU";

				const reqEl = cloneHidden(
					voxtralReqs.compatible ? "voice-voxtral-requirements-ok" : "voice-voxtral-requirements-fail",
				);
				if (reqEl) {
					const detectedEl = reqEl.querySelector("[data-voxtral-detected]") as HTMLElement;
					if (detectedEl) detectedEl.textContent = detected;
					if (!voxtralReqs.compatible && voxtralReqs.reasons?.length) {
						const ul = reqEl.querySelector("[data-voxtral-reasons]") as HTMLElement;
						for (const r of voxtralReqs.reasons) {
							const li = document.createElement("li");
							li.style.margin = "2px 0";
							li.textContent = r;
							ul.appendChild(li);
						}
					}
					reqsContainer.appendChild(reqEl);
				}
			} else {
				const loadingEl = document.createElement("div");
				loadingEl.className = "text-xs text-[var(--muted)] mb-3";
				loadingEl.textContent = "Checking system requirements\u2026";
				reqsContainer.appendChild(loadingEl);
			}
		}

		container.appendChild(el);
	}, [providerId, voxtralReqs]);

	return <div ref={ref} />;
}

// ── AddVoiceProviderModal ───────────────────────────────────

interface AddVoiceProviderModalProps {
	unconfiguredProviders: VoiceProviderData[];
	voxtralReqs: VoxtralRequirements | null;
	onSaved: () => void;
}

interface ElevenlabsCatalog {
	voices: { id: string; name: string }[];
	models: { id: string; name: string }[];
	warning: string | null;
}

export function AddVoiceProviderModal({
	unconfiguredProviders,
	voxtralReqs,
	onSaved,
}: AddVoiceProviderModalProps): VNode {
	const [apiKey, setApiKey] = useState("");
	const [baseUrlValue, setBaseUrlValue] = useState("");
	const [voiceValue, setVoiceValue] = useState("");
	const [modelValue, setModelValue] = useState("");
	const [languageCodeValue, setLanguageCodeValue] = useState("");
	const [elevenlabsCatalog, setElevenlabsCatalog] = useState<ElevenlabsCatalog>({
		voices: [],
		models: [],
		warning: null,
	});
	const [elevenlabsCatalogLoading, setElevenlabsCatalogLoading] = useState(false);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState("");

	const selectedProvider = voiceSelectedProvider.value;
	const providerMeta = selectedProvider
		? unconfiguredProviders.find((p) => p.id === selectedProvider) || voiceSelectedProviderData.value
		: null;
	const isElevenLabsProvider = selectedProvider === "elevenlabs" || selectedProvider === "elevenlabs-stt";
	const supportsTtsVoiceSettings = providerMeta?.type === "tts";
	const supportsBaseUrl = providerMeta?.capabilities?.baseUrl === true;

	function onClose(): void {
		voiceShowAddModal.value = false;
		voiceSelectedProvider.value = null;
		voiceSelectedProviderData.value = null;
		setApiKey("");
		setBaseUrlValue("");
		setVoiceValue("");
		setModelValue("");
		setLanguageCodeValue("");
		setError("");
	}

	function onSaveKey(): void {
		const hasApiKey = apiKey.trim().length > 0;
		const trimmedBaseUrl = baseUrlValue.trim();
		const hadBaseUrl =
			typeof providerMeta?.settings?.baseUrl === "string" && providerMeta.settings.baseUrl.trim().length > 0;
		const hasBaseUrl = supportsBaseUrl && (trimmedBaseUrl.length > 0 || hadBaseUrl);
		const hasSettings =
			(supportsTtsVoiceSettings && (voiceValue.trim() || modelValue.trim() || languageCodeValue.trim())) || hasBaseUrl;
		if (!(hasApiKey || hasSettings)) {
			setError("Provide an API key, base URL, or at least one provider setting.");
			return;
		}
		setError("");
		setSaving(true);

		const voiceOpts = {
			baseUrl: hasBaseUrl ? trimmedBaseUrl : undefined,
			voice: supportsTtsVoiceSettings ? voiceValue.trim() || undefined : undefined,
			model: supportsTtsVoiceSettings ? modelValue.trim() || undefined : undefined,
			languageCode: supportsTtsVoiceSettings ? languageCodeValue.trim() || undefined : undefined,
		};
		const req = hasApiKey
			? saveVoiceKey(selectedProvider as string, apiKey.trim(), voiceOpts)
			: saveVoiceSettings(selectedProvider as string, voiceOpts);
		req
			.then((r: unknown) => {
				const res = r as RpcResponse;
				setSaving(false);
				if (res?.ok) {
					setApiKey("");
					onSaved();
				} else {
					setError((res?.error as { message?: string })?.message || "Failed to save key");
				}
			})
			.catch((err: Error) => {
				setSaving(false);
				setError(err.message);
			});
	}

	function onSelectProvider(providerId: string): void {
		voiceSelectedProvider.value = providerId;
		voiceSelectedProviderData.value = null;
		setApiKey("");
		setBaseUrlValue("");
		setVoiceValue("");
		setModelValue("");
		setLanguageCodeValue("");
		setError("");
	}

	useEffect(() => {
		const settings = voiceSelectedProviderData.value?.settings;
		if (!settings) return;
		setBaseUrlValue(settings.baseUrl || "");
		setVoiceValue(settings.voiceId || settings.voice || "");
		setModelValue(settings.model || "");
		setLanguageCodeValue(settings.languageCode || "");
	}, [selectedProvider, voiceSelectedProviderData.value]);

	useEffect(() => {
		if (!isElevenLabsProvider) {
			setElevenlabsCatalog({ voices: [], models: [], warning: null });
			return;
		}
		setElevenlabsCatalogLoading(true);
		sendRpc("voice.elevenlabs.catalog", {})
			.then((res: RpcResponse) => {
				if (res?.ok) {
					const payload = res.payload as {
						voices?: { id: string; name: string }[];
						models?: { id: string; name: string }[];
						warning?: string;
					};
					setElevenlabsCatalog({
						voices: payload?.voices || [],
						models: payload?.models || [],
						warning: payload?.warning || null,
					});
				}
			})
			.catch(() => {
				setElevenlabsCatalog({ voices: [], models: [], warning: "Failed to fetch ElevenLabs voice catalog." });
			})
			.finally(() => {
				setElevenlabsCatalogLoading(false);
				rerender();
			});
	}, [selectedProvider, isElevenLabsProvider]);

	const sttCloud = unconfiguredProviders.filter((p) => p.type === "stt" && p.category === "cloud");
	const sttLocal = unconfiguredProviders.filter((p) => p.type === "stt" && p.category === "local");
	const ttsProviders = unconfiguredProviders.filter((p) => p.type === "tts");

	if (selectedProvider && providerMeta) {
		if (providerMeta.category === "cloud") {
			return (
				<Modal show={voiceShowAddModal.value} onClose={onClose} title={`Add ${providerMeta.name}`}>
					<div className="channel-form">
						<div className="text-sm text-[var(--text-strong)]">{providerMeta.name}</div>
						<div className="mb-3 text-xs text-[var(--muted)]">{providerMeta.description}</div>

						<label className="text-xs text-[var(--muted)]">API Key</label>
						<input
							type="password"
							className="provider-key-input w-full"
							value={apiKey}
							onInput={(e: Event) => setApiKey(targetValue(e))}
							placeholder={providerMeta.keyPlaceholder || "Leave blank to keep existing key"}
						/>
						{providerMeta.keyUrl ? (
							<div className="text-xs text-[var(--muted)]">
								Get your API key at{" "}
								<a
									href={providerMeta.keyUrl}
									target="_blank"
									rel="noopener"
									className="hover:underline text-[var(--accent)]"
								>
									{providerMeta.keyUrlLabel}
								</a>
							</div>
						) : null}

						{supportsBaseUrl ? (
							<div className="mt-2 flex flex-col gap-2">
								<label className="text-xs text-[var(--muted)]">Base URL</label>
								<input
									type="text"
									className="provider-key-input w-full"
									data-field="baseUrl"
									value={baseUrlValue}
									onInput={(e: Event) => setBaseUrlValue(targetValue(e))}
									placeholder="http://localhost:8000/v1"
								/>
								<div className="text-xs text-[var(--muted)]">
									Use this for a local or OpenAI-compatible server. Leave the API key blank if your endpoint does not
									require one.
								</div>
							</div>
						) : null}

						{supportsTtsVoiceSettings ? (
							<div className="flex flex-col gap-2">
								<label className="text-xs text-[var(--muted)]">Voice</label>
								{isElevenLabsProvider && elevenlabsCatalogLoading ? (
									<div className="text-xs text-[var(--muted)]">Loading ElevenLabs voices...</div>
								) : null}
								{isElevenLabsProvider && elevenlabsCatalog.warning ? (
									<div className="text-xs text-[var(--muted)]">{elevenlabsCatalog.warning}</div>
								) : null}
								{isElevenLabsProvider && elevenlabsCatalog.voices.length > 0 ? (
									<select className="provider-key-input w-full" onChange={(e: Event) => setVoiceValue(targetValue(e))}>
										<option value="">Pick a voice from your account...</option>
										{elevenlabsCatalog.voices.map((v) => (
											<option key={v.id} value={v.id}>
												{v.name} ({v.id})
											</option>
										))}
									</select>
								) : null}
								<input
									type="text"
									className="provider-key-input w-full"
									value={voiceValue}
									onInput={(e: Event) => setVoiceValue(targetValue(e))}
									list={isElevenLabsProvider ? "elevenlabs-voice-options" : undefined}
									placeholder="voice id / name (optional)"
								/>
								{isElevenLabsProvider ? (
									<datalist id="elevenlabs-voice-options">
										{elevenlabsCatalog.voices.map((v) => (
											<option key={v.id} value={v.id}>
												{v.name}
											</option>
										))}
									</datalist>
								) : null}

								<label className="text-xs text-[var(--muted)]">Model</label>
								{isElevenLabsProvider && elevenlabsCatalog.models.length > 0 ? (
									<select className="provider-key-input w-full" onChange={(e: Event) => setModelValue(targetValue(e))}>
										<option value="">Pick a model...</option>
										{elevenlabsCatalog.models.map((m) => (
											<option key={m.id} value={m.id}>
												{m.name} ({m.id})
											</option>
										))}
									</select>
								) : null}
								<input
									type="text"
									className="provider-key-input w-full"
									value={modelValue}
									onInput={(e: Event) => setModelValue(targetValue(e))}
									list={isElevenLabsProvider ? "elevenlabs-model-options" : undefined}
									placeholder="model (optional)"
								/>
								{isElevenLabsProvider ? (
									<datalist id="elevenlabs-model-options">
										{elevenlabsCatalog.models.map((m) => (
											<option key={m.id} value={m.id}>
												{m.name}
											</option>
										))}
									</datalist>
								) : null}

								{selectedProvider === "google" || selectedProvider === "google-tts" ? (
									<div className="flex flex-col gap-2">
										<label className="text-xs text-[var(--muted)]">Language Code</label>
										<input
											type="text"
											className="provider-key-input w-full"
											value={languageCodeValue}
											onInput={(e: Event) => setLanguageCodeValue(targetValue(e))}
											placeholder="en-US (optional)"
										/>
									</div>
								) : null}
							</div>
						) : null}

						{providerMeta.hint ? (
							<div
								className="text-xs text-[var(--muted)]"
								style={{
									marginTop: "8px",
									padding: "8px",
									background: "var(--surface-alt)",
									borderRadius: "4px",
									fontStyle: "italic",
								}}
							>
								{providerMeta.hint}
							</div>
						) : null}

						{error ? (
							<div className="text-xs" style={{ color: "var(--error)" }}>
								{error}
							</div>
						) : null}

						<div style={{ display: "flex", gap: "8px", marginTop: "8px" }}>
							<button
								className="provider-btn provider-btn-secondary"
								onClick={() => {
									voiceSelectedProvider.value = null;
									setApiKey("");
									setError("");
								}}
							>
								Back
							</button>
							<button className="provider-btn" disabled={saving} onClick={onSaveKey}>
								{saving ? "Saving\u2026" : "Save"}
							</button>
						</div>
					</div>
				</Modal>
			);
		}

		if (providerMeta.category === "local") {
			return (
				<Modal show={voiceShowAddModal.value} onClose={onClose} title={`Add ${providerMeta.name}`}>
					<div className="channel-form">
						<div className="text-sm text-[var(--text-strong)]">{providerMeta.name}</div>
						<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "12px" }}>
							{providerMeta.description}
						</div>
						<LocalProviderInstructions providerId={selectedProvider} voxtralReqs={voxtralReqs} />
						<div style={{ display: "flex", gap: "8px", marginTop: "12px" }}>
							<button
								className="provider-btn provider-btn-secondary"
								onClick={() => {
									voiceSelectedProvider.value = null;
								}}
							>
								Back
							</button>
						</div>
					</div>
				</Modal>
			);
		}
	}

	const providerButton = (p: VoiceProviderData) => (
		<button
			key={p.id}
			className="provider-card"
			style={{
				padding: "10px 12px",
				borderRadius: "6px",
				cursor: "pointer",
				textAlign: "left",
				border: "1px solid var(--border)",
				background: "var(--surface)",
			}}
			onClick={() => onSelectProvider(p.id)}
		>
			<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
				<div style={{ flex: 1 }}>
					<div className="text-sm text-[var(--text-strong)]">{p.name}</div>
					<div className="text-xs text-[var(--muted)]">{p.description}</div>
				</div>
				<span className="icon icon-chevron-right" style={{ color: "var(--muted)" }} />
			</div>
		</button>
	);

	return (
		<Modal show={voiceShowAddModal.value} onClose={onClose} title="Add Voice Provider">
			<div className="channel-form" style={{ gap: "16px" }}>
				{sttCloud.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Speech-to-Text (Cloud)
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>{sttCloud.map(providerButton)}</div>
					</div>
				) : null}

				{sttLocal.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Speech-to-Text (Local)
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>{sttLocal.map(providerButton)}</div>
					</div>
				) : null}

				{ttsProviders.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Text-to-Speech
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
							{ttsProviders.map(providerButton)}
						</div>
					</div>
				) : null}

				{unconfiguredProviders.length === 0 ? (
					<div className="text-sm text-[var(--muted)]" style={{ textAlign: "center", padding: "20px 0" }}>
						All available providers are already configured.
					</div>
				) : null}
			</div>
		</Modal>
	);
}
