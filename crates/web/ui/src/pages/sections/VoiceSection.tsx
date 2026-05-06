// ── Voice section ────────────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { TabBar } from "../../components/forms/Tabs";
import * as gon from "../../gon";
import { sendRpc } from "../../helpers";
import { connected } from "../../signals";
import * as S from "../../state";
import { fetchPhrase } from "../../tts-phrases";
import { targetChecked, targetValue } from "../../typed-events";
import { showToast } from "../../ui";
import { getPttKey, getVadSensitivity, setPttKey, setVadSensitivity } from "../../voice-input";
import {
	decodeBase64Safe,
	deleteVoicePersona,
	fetchVoiceProviders,
	listVoicePersonas,
	setActiveVoicePersona,
	testTts,
	testTtsWithPersona,
	toggleVoiceProvider,
	transcribeAudio,
	type VoicePersonaResponse,
} from "../../voice-utils";
import type { RpcResponse } from "./_shared";
import { rerender } from "./_shared";
import {
	AddVoiceProviderModal,
	PersonaEditModal,
	type VoiceProviderData,
	type VoiceTesting,
	type VoiceTestResult,
	type VoxtralRequirements,
	voiceSelectedProvider,
	voiceSelectedProviderData,
	voiceShowAddModal,
} from "./VoiceModals";

interface VoiceProviders {
	tts: VoiceProviderData[];
	stt: VoiceProviderData[];
}

interface PttKeyPickerProps {
	pttListening: boolean;
	setPttListening: (v: boolean) => void;
	pttKeyValue: string;
	setPttKeyValue: (v: string) => void;
}

function PttKeyPicker({ pttListening, setPttListening, pttKeyValue, setPttKeyValue }: PttKeyPickerProps): VNode {
	const handlerRef = useRef<((ev: KeyboardEvent) => void) | null>(null);

	useEffect(() => {
		return () => {
			if (handlerRef.current) {
				document.removeEventListener("keydown", handlerRef.current, true);
				handlerRef.current = null;
			}
		};
	}, []);

	return (
		<button
			type="button"
			className="provider-key-input"
			style={{ minWidth: "120px", textAlign: "center", cursor: "pointer" }}
			onClick={() => {
				if (pttListening) return;
				setPttListening(true);
				const handler = (ev: KeyboardEvent): void => {
					ev.preventDefault();
					ev.stopPropagation();
					setPttKeyValue(ev.key);
					setPttKey(ev.key);
					setPttListening(false);
					document.removeEventListener("keydown", handler, true);
					handlerRef.current = null;
					rerender();
				};
				handlerRef.current = handler;
				document.addEventListener("keydown", handler, true);
				rerender();
			}}
		>
			{pttListening ? "Press any key..." : pttKeyValue}
		</button>
	);
}

export function VoiceSection(): VNode {
	const [allProviders, setAllProviders] = useState<VoiceProviders>({ tts: [], stt: [] });
	const [voiceLoading, setVoiceLoading] = useState(true);
	const [voxtralReqs, setVoxtralReqs] = useState<VoxtralRequirements | null>(null);
	const [savingProvider, setSavingProvider] = useState<string | null>(null);
	const [voiceTesting, setVoiceTesting] = useState<VoiceTesting | null>(null);
	const [activeRecorder, setActiveRecorder] = useState<MediaRecorder | null>(null);
	const [voiceTestResults, setVoiceTestResults] = useState<Record<string, VoiceTestResult>>({});

	// Tab state
	const [activeTab, setActiveTab] = useState("stt");

	// Per-persona test state: persona id → "testing" | "playing" | null
	const [personaTesting, setPersonaTesting] = useState<Record<string, string>>({});

	// Voice personas
	const [personas, setPersonas] = useState<VoicePersonaResponse[]>([]);
	const [personaEditing, setPersonaEditing] = useState<string | null>(null);

	// PTT key configuration
	const [pttKeyValue, setPttKeyValue] = useState(getPttKey());
	const [pttListening, setPttListening] = useState(false);

	// VAD sensitivity
	const [vadSens, setVadSens] = useState(getVadSensitivity());

	function fetchVoiceStatus(options?: { silent?: boolean }): void {
		if (!options?.silent) {
			setVoiceLoading(true);
			rerender();
		}
		Promise.all([fetchVoiceProviders(), sendRpc("voice.config.voxtral_requirements", {})])
			.then(([providers, voxtral]) => {
				const provRes = providers as RpcResponse;
				const voxtralRes = voxtral as RpcResponse;
				if (provRes?.ok) setAllProviders((provRes.payload as VoiceProviders) || { tts: [], stt: [] });
				if (voxtralRes?.ok) setVoxtralReqs(voxtralRes.payload as VoxtralRequirements);
				if (!options?.silent) setVoiceLoading(false);
				rerender();
			})
			.catch(() => {
				if (!options?.silent) setVoiceLoading(false);
				rerender();
			});
	}

	async function fetchPersonas(): Promise<void> {
		try {
			const result = await listVoicePersonas();
			setPersonas(result.personas || []);
		} catch (_err) {
			/* ignore */
		}
	}

	useEffect(() => {
		if (connected.value) {
			fetchVoiceStatus();
			fetchPersonas();
		}
	}, [connected.value]);

	function onToggleProvider(provider: VoiceProviderData, enabled: boolean, providerType: string): void {
		setSavingProvider(provider.id);
		rerender();

		toggleVoiceProvider(provider.id, enabled, providerType)
			.then((r: unknown) => {
				const res = r as RpcResponse;
				setSavingProvider(null);
				if (res?.ok) {
					showToast(`${provider.name} ${enabled ? "enabled" : "disabled"}.`, "success");
					fetchVoiceStatus({ silent: true });
				} else {
					showToast((res?.error as { message?: string })?.message || "Failed to toggle provider", "error");
				}
				rerender();
			})
			.catch((err: Error) => {
				setSavingProvider(null);
				showToast(err.message, "error");
				rerender();
			});
	}

	function onConfigureProvider(providerId: string, providerData: VoiceProviderData): void {
		voiceSelectedProvider.value = providerId;
		voiceSelectedProviderData.value = providerData || null;
		voiceShowAddModal.value = true;
	}

	function getUnconfiguredProviders(): VoiceProviderData[] {
		return [...allProviders.stt, ...allProviders.tts].filter((p) => !p.available);
	}

	function stopSttRecording(): void {
		if (activeRecorder) {
			activeRecorder.stop();
		}
	}

	function humanizeMicError(err: { name?: string; message?: string }): string {
		if (err.name === "OverconstrainedError" || (err.message && /constraint/i.test(err.message))) {
			return "No compatible microphone found. Check your audio input device.";
		}
		if (err.name === "NotFoundError" || err.name === "NotAllowedError") {
			return "Microphone access denied or no microphone found. Check browser permissions.";
		}
		if (err.name === "NotReadableError") {
			return "Microphone is in use by another application.";
		}
		return err.message || "STT test failed";
	}

	async function testVoiceProvider(providerId: string, type: string): Promise<void> {
		if (voiceTesting?.id === providerId && voiceTesting?.type === "stt" && voiceTesting?.phase === "recording") {
			stopSttRecording();
			return;
		}

		setVoiceTesting({ id: providerId, type, phase: "testing" });
		rerender();

		if (type === "tts") {
			try {
				const id = gon.get("identity") as { user_name?: string; name?: string } | undefined;
				const user = id?.user_name || "friend";
				const bot = id?.name || "Moltis";
				const ttsText = await fetchPhrase("settings", user, bot);
				const res = (await testTts(ttsText, providerId)) as RpcResponse;
				if (res?.ok && (res.payload as { audio?: string })?.audio) {
					const payload = res.payload as { audio: string; mimeType?: string; content_type?: string; format?: string };
					const bytes = decodeBase64Safe(payload.audio);
					const audioMime = payload.mimeType || payload.content_type || "audio/mpeg";
					const blob = new Blob([bytes as BlobPart], { type: audioMime });
					const url = URL.createObjectURL(blob);
					const audio = new Audio(url);
					audio.onerror = (e) => {
						console.error("[TTS] audio element error:", audio.error?.message || e);
						URL.revokeObjectURL(url);
					};
					audio.onended = () => URL.revokeObjectURL(url);
					audio.play().catch((e: Error) => console.error("[TTS] play() failed:", e));
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: true, error: null },
					}));
				} else {
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: false, error: (res?.error as { message?: string })?.message || "TTS test failed" },
					}));
				}
			} catch (err) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: { success: false, error: (err as Error).message || "TTS test failed" },
				}));
			}
			setVoiceTesting(null);
		} else {
			try {
				const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
				const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
					? "audio/webm;codecs=opus"
					: "audio/webm";
				const mediaRecorder = new MediaRecorder(stream, { mimeType });
				const audioChunks: Blob[] = [];

				mediaRecorder.ondataavailable = (e: BlobEvent) => {
					if (e.data.size > 0) audioChunks.push(e.data);
				};

				mediaRecorder.start();
				setActiveRecorder(mediaRecorder);
				setVoiceTesting({ id: providerId, type, phase: "recording" });
				rerender();

				mediaRecorder.onstop = async () => {
					setActiveRecorder(null);
					for (const track of stream.getTracks()) track.stop();
					setVoiceTesting({ id: providerId, type, phase: "transcribing" });
					rerender();

					const audioBlob = new Blob(audioChunks, { type: mediaRecorder.mimeType || mimeType });

					try {
						const resp = await transcribeAudio(S.activeSessionKey, providerId, audioBlob);
						if (resp.ok) {
							const sttRes = (await resp.json()) as {
								ok?: boolean;
								transcription?: { text?: string };
								transcriptionError?: string;
								error?: string;
							};

							if (sttRes.ok && typeof sttRes.transcription?.text === "string") {
								const transcriptText = sttRes.transcription.text.trim();
								setVoiceTestResults((prev) => ({
									...prev,
									[providerId]: {
										text: transcriptText || null,
										error: transcriptText ? null : "No speech detected",
									},
								}));
							} else {
								setVoiceTestResults((prev) => ({
									...prev,
									[providerId]: {
										text: null,
										error: sttRes.transcriptionError || sttRes.error || "STT test failed",
									},
								}));
							}
						} else {
							const errBody = await resp.text();
							console.error("[STT] upload failed: status=%d body=%s", resp.status, errBody);
							let errMsg = "STT test failed";
							try {
								errMsg = (JSON.parse(errBody) as { error?: string })?.error || errMsg;
							} catch (_e) {
								// not JSON
							}
							setVoiceTestResults((prev) => ({
								...prev,
								[providerId]: { text: null, error: `${errMsg} (HTTP ${resp.status})` },
							}));
						}
					} catch (fetchErr) {
						setVoiceTestResults((prev) => ({
							...prev,
							[providerId]: { text: null, error: (fetchErr as Error).message || "STT test failed" },
						}));
					}
					setVoiceTesting(null);
					rerender();
				};
			} catch (err) {
				showToast(humanizeMicError(err as { name?: string; message?: string }), "error");
				setVoiceTesting(null);
			}
		}
		rerender();
	}

	if (voiceLoading || !connected.value) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Voice</h2>
				<div className="text-xs text-[var(--muted)]">{connected.value ? "Loading\u2026" : "Connecting\u2026"}</div>
			</div>
		);
	}

	const voiceTabs = [
		{ id: "stt", label: "Speech-to-Text" },
		{ id: "tts", label: "Text-to-Speech" },
		{ id: "personas", label: "Voice Personas" },
		{ id: "input", label: "Input Settings" },
	];

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Voice</h2>

			<TabBar tabs={voiceTabs} active={activeTab} onChange={setActiveTab} />

			<div style={{ maxWidth: "700px", display: "flex", flexDirection: "column", gap: "16px" }}>
				{activeTab === "stt" && (
					<div className="flex flex-col gap-3">
						<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
							STT lets you use the microphone button in chat to record voice input.
						</p>
						{gon.get("stt_enabled") === false && (
							<div className="rounded border border-[var(--border-strong)] bg-[var(--surface2)] px-3 py-2 text-xs text-[var(--muted)]">
								Speech-to-text is disabled in your config (<code>voice.stt.enabled = false</code> in{" "}
								<code>moltis.toml</code>). Provider configuration is shown for reference.
							</div>
						)}
						<div className="flex flex-col gap-2">
							{allProviders.stt.map((prov) => {
								const testState = voiceTesting?.id === prov.id && voiceTesting?.type === "stt" ? voiceTesting : null;
								const testResult = voiceTestResults[prov.id] || null;
								return (
									<VoiceProviderRow
										key={prov.id}
										provider={prov}
										meta={prov}
										type="stt"
										saving={savingProvider === prov.id}
										testState={testState}
										testResult={testResult}
										onToggle={(enabled: boolean) => onToggleProvider(prov, enabled, "stt")}
										onConfigure={() => onConfigureProvider(prov.id, prov)}
										onTest={() => testVoiceProvider(prov.id, "stt")}
									/>
								);
							})}
						</div>
					</div>
				)}

				{activeTab === "tts" && (
					<div className="flex flex-col gap-3">
						<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
							TTS lets you hear responses as audio. Configure providers and test voices.
						</p>
						{gon.get("tts_enabled") === false && (
							<div className="rounded border border-[var(--border-strong)] bg-[var(--surface2)] px-3 py-2 text-xs text-[var(--muted)]">
								Text-to-speech is disabled in your config (<code>voice.tts.enabled = false</code> in{" "}
								<code>moltis.toml</code>). Provider configuration is shown for reference.
							</div>
						)}
						<div className="flex flex-col gap-2">
							{allProviders.tts.map((prov) => {
								const testState = voiceTesting?.id === prov.id && voiceTesting?.type === "tts" ? voiceTesting : null;
								const testResult = voiceTestResults[prov.id] || null;
								return (
									<VoiceProviderRow
										key={prov.id}
										provider={prov}
										meta={prov}
										type="tts"
										saving={savingProvider === prov.id}
										testState={testState}
										testResult={testResult}
										onToggle={(enabled: boolean) => onToggleProvider(prov, enabled, "tts")}
										onConfigure={() => onConfigureProvider(prov.id, prov)}
										onTest={() => testVoiceProvider(prov.id, "tts")}
										preferred={prov.preferred}
										onSetPreferred={() => {
											sendRpc("tts.setProvider", { provider: prov.id }).then(() => {
												fetchVoiceStatus({ silent: true });
												rerender();
											});
										}}
									/>
								);
							})}
						</div>
					</div>
				)}

				{activeTab === "personas" && (
					<div className="flex flex-col gap-3">
						<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
							Named voice identities injected into every TTS call. Instead of improvising tone per-message, a persona
							defines a stable spoken character.
						</p>
						{personas.length === 0 ? (
							<p className="text-xs text-[var(--muted)] italic">No personas configured yet.</p>
						) : (
							<div className="flex flex-col gap-2">
								{personas.map((pr) => (
									<div
										key={pr.persona.id}
										className={`flex items-center gap-3 p-3 rounded border ${pr.isActive ? "border-[var(--accent)]" : "border-[var(--border)]"}`}
										style={{ background: "var(--surface)" }}
									>
										<div className="flex-1 min-w-0">
											<div className="flex items-center gap-2 flex-wrap">
												<span className="text-sm font-medium text-[var(--text-strong)]">{pr.persona.label}</span>
												{pr.isActive ? (
													<span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--accent)] text-white">
														active
													</span>
												) : null}
												{(pr.persona.provider_bindings || []).map((b) => (
													<span
														key={b.provider}
														className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--surface-alt)] text-[var(--muted)]"
													>
														{b.provider}
														{b.voice_id ? `: ${b.voice_id}` : ""}
													</span>
												))}
											</div>
											{pr.persona.description ? (
												<p className="text-xs text-[var(--muted)] truncate" style={{ margin: "2px 0 0 0" }}>
													{pr.persona.description}
												</p>
											) : null}
											{pr.persona.prompt.profile ? (
												<p className="text-[10px] text-[var(--muted)] truncate italic" style={{ margin: "2px 0 0 0" }}>
													{pr.persona.prompt.profile}
												</p>
											) : null}
										</div>
										<div className="flex items-center gap-1.5">
											<button
												type="button"
												className="provider-btn provider-btn-secondary text-xs !py-1 !px-2.5"
												disabled={!!personaTesting[pr.persona.id]}
												onClick={async () => {
													setPersonaTesting((prev) => ({ ...prev, [pr.persona.id]: "testing" }));
													rerender();
													try {
														const identity = gon.get("identity") as { user_name?: string; name?: string } | undefined;
														const user = identity?.user_name || "friend";
														const bot = identity?.name || "Moltis";
														const text = await fetchPhrase("settings", user, bot);
														const res = (await testTtsWithPersona(text, pr.persona.id)) as RpcResponse;
														if (res?.ok) {
															const payload = res.payload as {
																audio?: string;
																mimeType?: string;
															};
															if (payload?.audio) {
																setPersonaTesting((prev) => ({ ...prev, [pr.persona.id]: "playing" }));
																rerender();
																const bytes = decodeBase64Safe(payload.audio);
																const blob = new Blob([bytes as BlobPart], {
																	type: payload.mimeType || "audio/mpeg",
																});
																const url = URL.createObjectURL(blob);
																const audio = new Audio(url);
																audio.onended = () => {
																	URL.revokeObjectURL(url);
																	setPersonaTesting((prev) => ({ ...prev, [pr.persona.id]: "" }));
																	rerender();
																};
																audio.play().catch((e: Error) => console.error("[TTS]", e));
																return;
															}
														}
													} catch (_e) {
														/* ignore */
													}
													setPersonaTesting((prev) => ({ ...prev, [pr.persona.id]: "" }));
													rerender();
												}}
											>
												{personaTesting[pr.persona.id] === "testing"
													? "Testing\u2026"
													: personaTesting[pr.persona.id] === "playing"
														? "Playing\u2026"
														: "Test"}
											</button>
											<button
												type="button"
												className="provider-btn provider-btn-secondary text-xs !py-1 !px-2.5"
												onClick={() => setPersonaEditing(pr.persona.id)}
											>
												Edit
											</button>
											{pr.isActive ? (
												<button
													type="button"
													className="provider-btn provider-btn-secondary text-xs !py-1 !px-2.5"
													onClick={async () => {
														await setActiveVoicePersona(null);
														fetchPersonas();
													}}
												>
													Deactivate
												</button>
											) : (
												<button
													type="button"
													className="provider-btn provider-btn-secondary text-xs !py-1 !px-2.5"
													onClick={async () => {
														await setActiveVoicePersona(pr.persona.id);
														fetchPersonas();
													}}
												>
													Activate
												</button>
											)}
											<button
												type="button"
												className="provider-btn text-xs !py-1 !px-2.5 !bg-[var(--error)] hover:!bg-red-700"
												onClick={async () => {
													await deleteVoicePersona(pr.persona.id);
													fetchPersonas();
												}}
											>
												Remove
											</button>
										</div>
									</div>
								))}
							</div>
						)}
						<button type="button" className="provider-btn" onClick={() => setPersonaEditing("__new__")}>
							+ Add Persona
						</button>

						{personaEditing !== null ? (
							<PersonaEditModal
								editingId={personaEditing}
								existingPersona={
									personaEditing !== "__new__" ? (personas.find((p) => p.persona.id === personaEditing) ?? null) : null
								}
								onClose={() => setPersonaEditing(null)}
								onSaved={() => {
									setPersonaEditing(null);
									fetchPersonas();
								}}
							/>
						) : null}
					</div>
				)}

				{activeTab === "input" && (
					<div className="flex flex-col gap-6">
						<div className="flex flex-col gap-3">
							<h3 className="text-sm font-medium text-[var(--text-strong)]">Push-to-Talk</h3>
							<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
								Hold a keyboard key to record voice input. Release to send. Function keys (F1–F24) work even when
								focused in an input field.
							</p>
							<div className="flex items-center gap-3">
								<span className="text-xs text-[var(--muted)]">PTT Key:</span>
								<PttKeyPicker
									pttListening={pttListening}
									setPttListening={setPttListening}
									pttKeyValue={pttKeyValue}
									setPttKeyValue={setPttKeyValue}
								/>
							</div>
						</div>

						<div className="flex flex-col gap-3">
							<h3 className="text-sm font-medium text-[var(--text-strong)]">Conversation Mode (VAD)</h3>
							<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
								Adjust how sensitive the voice activity detection is. Higher values pick up softer speech but may
								trigger on background noise.
							</p>
							<div className="flex items-center gap-3">
								<span className="text-xs text-[var(--muted)]" style={{ minWidth: "80px" }}>
									Sensitivity:
								</span>
								<input
									type="range"
									min="0"
									max="100"
									step="5"
									value={vadSens}
									style={{ flex: 1, maxWidth: "200px", accentColor: "var(--accent)" }}
									onInput={(e) => {
										const val = parseInt(targetValue(e), 10);
										setVadSens(val);
										setVadSensitivity(val);
										rerender();
									}}
								/>
								<span className="text-xs text-[var(--muted)]" style={{ minWidth: "35px", textAlign: "right" }}>
									{vadSens}%
								</span>
							</div>
						</div>
					</div>
				)}
			</div>

			<AddVoiceProviderModal
				unconfiguredProviders={getUnconfiguredProviders()}
				voxtralReqs={voxtralReqs}
				onSaved={() => {
					fetchVoiceStatus();
					voiceShowAddModal.value = false;
					voiceSelectedProvider.value = null;
					voiceSelectedProviderData.value = null;
				}}
			/>
		</div>
	);
}

// Individual provider row with enable toggle

interface VoiceProviderRowProps {
	provider: VoiceProviderData;
	meta: VoiceProviderData;
	type: string;
	saving: boolean;
	testState: VoiceTesting | null;
	testResult: VoiceTestResult | null;
	onToggle: (enabled: boolean) => void;
	onConfigure: () => void;
	onTest: () => void;
	preferred?: boolean;
	onSetPreferred?: () => void;
}

function VoiceProviderRow({
	provider,
	meta,
	type,
	saving,
	testState,
	testResult,
	onToggle,
	onConfigure,
	onTest,
	preferred,
	onSetPreferred,
}: VoiceProviderRowProps): VNode {
	const canEnable = provider.available;
	const keySourceLabel =
		provider.keySource === "env" ? "(from env)" : provider.keySource === "llm_provider" ? "(from LLM provider)" : "";
	const showTestBtn = canEnable && provider.enabled;

	let buttonText = "Test";
	let buttonDisabled = false;
	if (testState) {
		if (testState.phase === "recording") {
			buttonText = "Stop";
		} else if (testState.phase === "transcribing") {
			buttonText = "Testing\u2026";
			buttonDisabled = true;
		} else {
			buttonText = "Testing\u2026";
			buttonDisabled = true;
		}
	}

	return (
		<div
			className="provider-card"
			style={{ padding: "10px 14px", borderRadius: "8px", display: "flex", alignItems: "center", gap: "12px" }}
		>
			<div style={{ flex: 1, display: "flex", flexDirection: "column", gap: "2px" }}>
				<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
					<span className="text-sm text-[var(--text-strong)]">{meta.name}</span>
					{preferred ? (
						<span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--accent)] text-white">preferred</span>
					) : null}
					{provider.category === "local" ? <span className="provider-item-badge">local</span> : null}
					{keySourceLabel ? <span className="text-xs text-[var(--muted)]">{keySourceLabel}</span> : null}
				</div>
				<span className="text-xs text-[var(--muted)]">{meta.description}</span>
				{provider.settingsSummary ? (
					<span className="text-xs text-[var(--muted)]">Voice: {provider.settingsSummary}</span>
				) : null}
				{provider.binaryPath ? (
					<span className="text-xs text-[var(--muted)]">Found at: {provider.binaryPath}</span>
				) : null}
				{!canEnable && provider.statusMessage ? (
					<span className="text-xs text-[var(--muted)]">{provider.statusMessage}</span>
				) : null}
				{testState?.phase === "recording" ? (
					<div className="voice-recording-hint">
						<span className="voice-recording-dot" />
						<span>Speak now, then click Stop when finished</span>
					</div>
				) : null}
				{testState?.phase === "transcribing" ? (
					<span className="text-xs text-[var(--muted)]">Transcribing...</span>
				) : null}
				{testState?.phase === "testing" && type === "tts" ? (
					<span className="text-xs text-[var(--muted)]">Playing audio...</span>
				) : null}
				{testResult?.text ? (
					<div className="voice-transcription-result">
						<span className="voice-transcription-label">Transcribed:</span>
						<span className="voice-transcription-text">"{testResult.text}"</span>
					</div>
				) : null}
				{testResult?.success === true ? (
					<div className="voice-success-result">
						<span className="icon icon-md icon-check-circle" />
						<span>Audio played successfully</span>
					</div>
				) : null}
				{testResult?.error ? (
					<div className="voice-error-result">
						<span className="icon icon-md icon-x-circle" />
						<span>{testResult.error}</span>
					</div>
				) : null}
			</div>
			<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
				{onSetPreferred && provider.enabled && !preferred ? (
					<button
						className="provider-btn provider-btn-secondary text-xs !py-1 !px-2"
						onClick={onSetPreferred}
						title="Set as preferred TTS provider"
					>
						📌
					</button>
				) : null}
				<button className="provider-btn provider-btn-secondary provider-btn-sm" onClick={onConfigure}>
					Configure
				</button>
				{showTestBtn ? (
					<button
						className="provider-btn provider-btn-secondary provider-btn-sm"
						onClick={onTest}
						disabled={buttonDisabled}
						title={type === "tts" ? "Test voice output" : "Test voice input"}
					>
						{buttonText}
					</button>
				) : null}
				{canEnable ? (
					<label className="toggle-switch">
						<input
							type="checkbox"
							checked={provider.enabled}
							disabled={saving}
							onChange={(e: Event) => onToggle(targetChecked(e))}
						/>
						<span className="toggle-slider" />
					</label>
				) : provider.category === "local" ? (
					<span className="text-xs text-[var(--muted)]">Install required</span>
				) : null}
			</div>
		</div>
	);
}
