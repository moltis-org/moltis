// ── Nodes page ──────────────────────────────────────────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { onEvent } from "../events";
import { sendRpc } from "../helpers";
import { navigate } from "../router";
import { settingsPath } from "../routes";
import { ConfirmDialog, copyToClipboard, requestConfirm } from "../ui";

// ── Types ────────────────────────────────────────────────────

interface NodeInfo {
	nodeId: string;
	displayName?: string;
	platform?: string;
	version?: string;
	remoteIp?: string;
	capabilities?: string[];
	telemetry?: TelemetryData;
}

interface TelemetryData {
	cpuCount?: number;
	cpuUsage?: number;
	memTotal?: number;
	memAvailable?: number;
	uptimeSecs?: number;
	services?: unknown[];
	stale?: boolean;
}

interface PendingPair {
	id: string;
	deviceId: string;
	displayName?: string;
	platform?: string;
	fingerprint?: string;
}

interface PairedDevice {
	deviceId: string;
	displayName?: string;
	platform?: string;
	publicKey?: string;
	fingerprint?: string;
	createdAt?: string;
}

interface PairingStatus {
	enabled: boolean;
}

interface DoctorSnapshot {
	exec_host?: string;
	active_route?: {
		label?: string;
		target?: string;
		target_id?: string;
		port?: number;
		host_pinned?: boolean;
	};
	checks?: DoctorCheck[];
	paired_node_count?: number;
	managed_target_count?: number;
	pinned_target_count?: number;
	managed_key_count?: number;
	encrypted_key_count?: number;
}

interface DoctorCheck {
	title: string;
	level: string;
	message: string;
	hint?: string;
}

interface DoctorTestResult {
	route_label?: string;
	reachable: boolean;
	exit_code?: number;
	failure_hint?: string;
	stderr?: string;
}

interface ToastItem {
	id: number;
	message: string;
	type: string;
}

// ── Signals ─────────────────────────────────────────────────
const nodes = signal<NodeInfo[]>([]);
const pendingPairs = signal<PendingPair[]>([]);
const pairedDevices = signal<PairedDevice[]>([]);
const pairingEnabled = signal(false);
const pairingStatusLoading = signal(false);
const loading = signal(false);
const activeTab = signal<"connected" | "paired" | "pending">("connected");
const toasts = signal<ToastItem[]>([]);
let toastId = 0;
const doctor = signal<DoctorSnapshot | null>(null);
const doctorLoading = signal(false);
const doctorError = signal("");
const doctorTest = signal<DoctorTestResult | null>(null);
const doctorTestLoading = signal(false);
const doctorPinLoading = signal(false);

// ── Helpers ─────────────────────────────────────────────────

function isSshTargetNode(node: NodeInfo): boolean {
	return node?.platform === "ssh" || String(node?.nodeId || "").startsWith("ssh:");
}

function sshTargetValue(node: NodeInfo): string {
	if (String(node.nodeId || "").startsWith("ssh:")) return String(node.nodeId).slice(4);
	return String(node.displayName || "")
		.replace(/^SSH:\s*/i, "")
		.trim();
}

function nodeDisplayLabel(node: NodeInfo | null): string {
	if (!node) return "Local";
	if (isSshTargetNode(node)) {
		const target = sshTargetValue(node);
		return target ? `SSH: ${target}` : node.displayName || node.nodeId;
	}
	return node.displayName || node.nodeId;
}

function gatewayWsUrl(): string {
	const proto = location.protocol === "https:" ? "wss:" : "ws:";
	const host = location.hostname;
	const port = location.port;
	return `${proto}//${host}${port ? `:${port}` : ""}/ws`;
}

function showToast(message: string, type: string): void {
	const id = ++toastId;
	toasts.value = toasts.value.concat([{ id, message, type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((t) => t.id !== id);
	}, 4000);
}

async function refreshNodes(): Promise<void> {
	loading.value = true;
	try {
		const res = await sendRpc<NodeInfo[]>("node.list", {});
		if (res?.ok) nodes.value = res.payload || [];
	} catch {
		/* ignore */
	}
	loading.value = false;
}

async function refreshPendingPairs(): Promise<void> {
	try {
		const res = await sendRpc<PendingPair[]>("node.pair.list", {});
		if (res?.ok) pendingPairs.value = res.payload || [];
	} catch {
		/* ignore */
	}
}

async function refreshPairedDevices(): Promise<void> {
	try {
		const res = await sendRpc<PairedDevice[]>("device.pair.list", {});
		if (res?.ok) pairedDevices.value = res.payload || [];
	} catch {
		/* ignore */
	}
}

async function refreshPairingStatus(): Promise<void> {
	pairingStatusLoading.value = true;
	try {
		const res = await sendRpc<PairingStatus>("node.pairing.status", {});
		if (res?.ok) pairingEnabled.value = Boolean(res.payload?.enabled);
	} catch {
		/* ignore */
	} finally {
		pairingStatusLoading.value = false;
	}
}

async function refreshDoctor(): Promise<void> {
	doctorLoading.value = true;
	doctorError.value = "";
	try {
		const response = await fetch("/api/ssh/doctor");
		if (!response.ok) throw new Error("Failed to load remote exec status");
		doctor.value = await response.json();
	} catch (err) {
		doctorError.value = (err as Error).message || "Failed to load remote exec status";
	} finally {
		doctorLoading.value = false;
	}
}

async function testActiveSshRoute(): Promise<void> {
	doctorTestLoading.value = true;
	doctorError.value = "";
	try {
		const response = await fetch("/api/ssh/doctor/test-active", { method: "POST" });
		const data = await response.json();
		if (!response.ok) throw new Error(data?.error || "Failed to test SSH route");
		doctorTest.value = data;
		showToast(
			data.reachable ? "Active SSH route is reachable" : data.failure_hint || "Active SSH route check failed",
			data.reachable ? "success" : "error",
		);
	} catch (err) {
		doctorError.value = (err as Error).message || "Failed to test SSH route";
		showToast(doctorError.value, "error");
	} finally {
		doctorTestLoading.value = false;
	}
}

async function repairActiveRouteHostPin(): Promise<void> {
	const snapshot = doctor.value;
	const activeRoute = snapshot?.active_route || null;
	if (!activeRoute?.target_id) {
		showToast("The active SSH route cannot be managed from the doctor panel", "error");
		return;
	}
	doctorPinLoading.value = true;
	doctorError.value = "";
	try {
		const scanResponse = await fetch("/api/ssh/host-key/scan", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ target: activeRoute.target, port: activeRoute.port ?? null }),
		});
		const scanData = await scanResponse.json();
		if (!scanResponse.ok) throw new Error(scanData?.error || "Failed to scan SSH host key");
		const pinResponse = await fetch(`/api/ssh/targets/${activeRoute.target_id}/pin`, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ known_host: scanData.known_host }),
		});
		const pinData = await pinResponse.json();
		if (!pinResponse.ok) throw new Error(pinData?.error || "Failed to pin SSH host key");
		await refreshDoctor();
		showToast(activeRoute.host_pinned ? "Active SSH host pin refreshed" : "Active SSH host pinned", "success");
	} catch (err) {
		doctorError.value = (err as Error).message || "Failed to repair SSH host pin";
		showToast(doctorError.value, "error");
	} finally {
		doctorPinLoading.value = false;
	}
}

async function clearActiveRouteHostPin(): Promise<void> {
	const snapshot = doctor.value;
	const activeRoute = snapshot?.active_route || null;
	if (!activeRoute?.target_id) {
		showToast("The active SSH route cannot be managed from the doctor panel", "error");
		return;
	}
	doctorPinLoading.value = true;
	doctorError.value = "";
	try {
		const response = await fetch(`/api/ssh/targets/${activeRoute.target_id}/pin`, { method: "DELETE" });
		const data = await response.json();
		if (!response.ok) throw new Error(data?.error || "Failed to clear SSH host pin");
		await refreshDoctor();
		showToast("Active SSH host pin cleared", "success");
	} catch (err) {
		doctorError.value = (err as Error).message || "Failed to clear SSH host pin";
		showToast(doctorError.value, "error");
	} finally {
		doctorPinLoading.value = false;
	}
}

async function refreshAll(): Promise<void> {
	await Promise.all([
		refreshNodes(),
		refreshPendingPairs(),
		refreshPairedDevices(),
		refreshPairingStatus(),
		refreshDoctor(),
	]);
}

async function setPairingEnabled(enabled: boolean): Promise<void> {
	pairingStatusLoading.value = true;
	const method = enabled ? "node.pairing.enable" : "node.pairing.disable";
	try {
		const res = await sendRpc<PairingStatus>(method, {});
		if (res?.ok) {
			pairingEnabled.value = Boolean(res.payload?.enabled);
			showToast(pairingEnabled.value ? "Node pairing enabled" : "Node pairing disabled", "success");
		} else {
			showToast(res?.error?.message || "Failed to update node pairing", "error");
		}
	} catch (err) {
		showToast(err instanceof Error ? err.message : "Failed to update node pairing", "error");
	} finally {
		pairingStatusLoading.value = false;
	}
}

async function approvePair(id: string): Promise<void> {
	const res = await sendRpc("node.pair.approve", { id });
	if (res?.ok) {
		showToast("Pairing approved -- device token issued", "success");
		await refreshAll();
	} else showToast(res?.error?.message || "Failed to approve", "error");
}

async function rejectPair(id: string): Promise<void> {
	const res = await sendRpc("node.pair.reject", { id });
	if (res?.ok) {
		showToast("Pairing rejected", "success");
		await refreshAll();
	} else showToast(res?.error?.message || "Failed to reject", "error");
}

async function revokeDevice(deviceId: string): Promise<void> {
	const ok = await requestConfirm(
		`Revoke device "${deviceId}"? This will disconnect the device and invalidate its token.`,
	);
	if (!ok) return;
	const res = await sendRpc("device.token.revoke", { deviceId });
	if (res?.ok) {
		showToast("Device token revoked", "success");
		await refreshAll();
	} else showToast(res?.error?.message || "Failed to revoke", "error");
}

// ── Components ──────────────────────────────────────────────

function TabBar(): VNode {
	const tabs = [
		{ id: "connected" as const, label: "Connected", count: nodes.value.length },
		{ id: "paired" as const, label: "Paired Devices", count: pairedDevices.value.length },
		{ id: "pending" as const, label: "Pending", count: pendingPairs.value.length },
	];
	return (
		<div className="flex gap-1 mb-4">
			{tabs.map((tab) => (
				<button
					type="button"
					key={tab.id}
					className={`px-3 py-1.5 text-sm rounded-md transition-colors ${activeTab.value === tab.id ? "bg-[var(--accent)] text-white" : "bg-[var(--surface-alt)] text-[var(--text-muted)] hover:bg-[var(--hover)]"}`}
					onClick={() => {
						activeTab.value = tab.id;
					}}
				>
					{tab.label}
					{tab.count > 0 ? <span className="ml-1 opacity-70">({tab.count})</span> : null}
				</button>
			))}
		</div>
	);
}

function formatBytes(bytes: number | null | undefined): string | null {
	if (bytes == null) return null;
	const gb = bytes / 1073741824;
	if (gb >= 1) return `${gb.toFixed(1)} GB`;
	return `${(bytes / 1048576).toFixed(0)} MB`;
}

function TelemetryBar({
	label,
	value,
	max,
}: {
	label: string;
	value: number | null | undefined;
	max: number | null | undefined;
}): VNode | null {
	if (value == null || max == null || max === 0) return null;
	const pct = Math.min(100, Math.max(0, (value / max) * 100));
	const color = pct > 80 ? "bg-red-500" : pct > 60 ? "bg-yellow-500" : "bg-green-500";
	return (
		<div className="flex items-center gap-2 text-xs text-[var(--text-muted)]">
			<span className="w-8 shrink-0">{label}</span>
			<div className="flex-1 h-1.5 rounded bg-[var(--border)] overflow-hidden">
				<div className={`${color} h-full rounded`} style={{ width: `${pct.toFixed(1)}%` }} />
			</div>
			<span className="w-16 text-right shrink-0">{pct.toFixed(0)}%</span>
		</div>
	);
}

function NodeTelemetry({ telemetry }: { telemetry?: TelemetryData }): VNode | null {
	if (!telemetry) return null;
	const parts: VNode[] = [];
	if (telemetry.cpuCount != null) parts.push(<span>{telemetry.cpuCount} cores</span>);
	if (telemetry.memTotal != null) parts.push(<span>{formatBytes(telemetry.memTotal)} RAM</span>);
	if (telemetry.uptimeSecs != null) {
		const h = Math.floor(telemetry.uptimeSecs / 3600);
		const d = Math.floor(h / 24);
		parts.push(<span>up {d > 0 ? `${d}d ${h % 24}h` : `${h}h`}</span>);
	}
	if (telemetry.stale) parts.push(<span className="text-yellow-500">(stale)</span>);
	return (
		<div className="mt-1.5 flex flex-col gap-1">
			{telemetry.cpuUsage != null ? <TelemetryBar label="CPU" value={telemetry.cpuUsage} max={100} /> : null}
			{telemetry.memTotal != null && telemetry.memAvailable != null ? (
				<TelemetryBar label="MEM" value={telemetry.memTotal - telemetry.memAvailable} max={telemetry.memTotal} />
			) : null}
			{parts.length > 0 ? <div className="text-xs text-[var(--text-muted)] flex gap-2 flex-wrap">{parts}</div> : null}
		</div>
	);
}

function DoctorBadge({ level }: { level: string }): VNode {
	const tone =
		level === "error"
			? "bg-red-500/15 text-red-500"
			: level === "warn"
				? "bg-yellow-500/15 text-yellow-500"
				: "bg-green-500/15 text-green-500";
	return <span className={`text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded ${tone}`}>{level}</span>;
}

function ConnectNodeForm(): VNode {
	const wsUrl = gatewayWsUrl();
	const addCmd = `moltis node add --host ${wsUrl}`;
	const enableCmd = `moltis node pairing enable --host ${location.origin}`;
	const isPairingEnabled = pairingEnabled.value;
	return (
		<div className="rounded-lg border border-[var(--border)] bg-[var(--surface-alt)] p-4">
			<div className="flex items-start justify-between gap-3 mb-3">
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)] mb-1">Connect a Remote Node</h3>
					<div className="text-xs text-[var(--text-muted)]">
						Pairing is{" "}
						<span className={isPairingEnabled ? "text-green-500" : "text-yellow-500"}>
							{isPairingEnabled ? "enabled" : "disabled"}
						</span>
						.
					</div>
				</div>
				<button
					type="button"
					className={
						isPairingEnabled ? "provider-btn provider-btn-secondary provider-btn-sm" : "provider-btn provider-btn-sm"
					}
					onClick={() => setPairingEnabled(!isPairingEnabled)}
					disabled={pairingStatusLoading.value}
				>
					{pairingStatusLoading.value ? "Updating..." : isPairingEnabled ? "Disable Pairing" : "Enable Pairing"}
				</button>
			</div>
			<p className="text-xs text-[var(--text-muted)] mb-3">
				{isPairingEnabled
					? "Run this command on the remote machine. The node generates an Ed25519 keypair and waits for you to approve its fingerprint in the Pending tab."
					: "Enable pairing before running the node command, then disable it again after approval."}
			</p>
			{isPairingEnabled ? null : (
				<div className="flex items-center gap-2 mb-3">
					<code className="flex-1 text-xs bg-[var(--bg)] px-2 py-1.5 rounded border border-[var(--border)] break-all select-all">
						{enableCmd}
					</code>
					<button
						type="button"
						className="provider-btn provider-btn-secondary provider-btn-sm shrink-0"
						onClick={() =>
							copyToClipboard(enableCmd, "", "").then((ok) =>
								showToast(
									ok ? "Copied to clipboard" : "Could not copy — please copy manually.",
									ok ? "success" : "error",
								),
							)
						}
					>
						Copy
					</button>
				</div>
			)}
			<div className="flex items-center gap-2 mb-3">
				<code className="flex-1 text-xs bg-[var(--bg)] px-2 py-1.5 rounded border border-[var(--border)] break-all select-all">
					{addCmd}
				</code>
				<button
					type="button"
					className="provider-btn provider-btn-secondary provider-btn-sm shrink-0"
					onClick={() =>
						copyToClipboard(addCmd, "", "").then((ok) =>
							showToast(
								ok ? "Copied to clipboard" : "Could not copy — please copy manually.",
								ok ? "success" : "error",
							),
						)
					}
				>
					Copy
				</button>
			</div>
			<p className="text-xs text-[var(--text-muted)]">
				Replace the host with your public IP or domain if the remote machine cannot reach this address directly. Check
				the{" "}
				<button
					type="button"
					className="underline hover:text-[var(--text-strong)]"
					onClick={() => {
						activeTab.value = "pending";
					}}
				>
					Pending
				</button>{" "}
				tab to approve incoming pairing requests.
			</p>
		</div>
	);
}

function RemoteExecStatusCard(): VNode {
	const snapshot = doctor.value;
	const execHost = snapshot?.exec_host || "local";
	const activeRoute = snapshot?.active_route || null;
	const checkList = snapshot?.checks || [];
	const canManageActivePin = Boolean(activeRoute?.target_id);

	return (
		<div className="rounded-lg border border-[var(--border)] bg-[var(--surface-alt)] p-4 flex flex-col gap-3">
			<div className="flex items-start justify-between gap-3 flex-wrap">
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)] mb-1">Remote Exec Status</h3>
					<p className="text-xs text-[var(--text-muted)] m-0">
						Moltis is currently configured to run commands through{" "}
						<strong className="text-[var(--text-strong)]"> {execHost}</strong>
						{activeRoute ? (
							<>
								{" "}
								using <code>{activeRoute.label}</code>
							</>
						) : null}
						.
					</p>
					{activeRoute ? (
						<div className="text-xs text-[var(--text-muted)] mt-1">
							{activeRoute.host_pinned
								? "Active route is pinned to a stored host key."
								: canManageActivePin
									? "Active route is currently inheriting global known_hosts policy."
									: "Active route is not directly manageable here because it comes from legacy config."}
						</div>
					) : null}
				</div>
				<div className="flex gap-2 flex-wrap">
					<button
						type="button"
						className="provider-btn provider-btn-secondary provider-btn-sm"
						onClick={refreshDoctor}
						disabled={doctorLoading.value}
					>
						{doctorLoading.value ? "Refreshing..." : "Refresh Doctor"}
					</button>
					{execHost === "ssh" && activeRoute ? (
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={testActiveSshRoute}
							disabled={doctorTestLoading.value}
						>
							{doctorTestLoading.value ? "Testing..." : "Test Active SSH Route"}
						</button>
					) : null}
					{execHost === "ssh" && activeRoute && canManageActivePin ? (
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={repairActiveRouteHostPin}
							disabled={doctorPinLoading.value}
						>
							{doctorPinLoading.value
								? "Scanning..."
								: activeRoute.host_pinned
									? "Refresh Active Pin"
									: "Pin Active Route"}
						</button>
					) : null}
					{execHost === "ssh" && activeRoute?.host_pinned && canManageActivePin ? (
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={clearActiveRouteHostPin}
							disabled={doctorPinLoading.value}
						>
							{doctorPinLoading.value ? "Clearing..." : "Clear Active Pin"}
						</button>
					) : null}
					<button
						type="button"
						className="provider-btn provider-btn-secondary provider-btn-sm"
						onClick={() => navigate(settingsPath("ssh"))}
					>
						SSH Settings
					</button>
				</div>
			</div>
			<div className="grid gap-2 md:grid-cols-5">
				<div className="rounded border border-[var(--border)] bg-[var(--bg)] px-3 py-2">
					<div className="text-[10px] uppercase tracking-wide text-[var(--text-muted)]">Backend</div>
					<div className="text-sm text-[var(--text-strong)] mt-1">{execHost}</div>
				</div>
				<div className="rounded border border-[var(--border)] bg-[var(--bg)] px-3 py-2">
					<div className="text-[10px] uppercase tracking-wide text-[var(--text-muted)]">Paired Nodes</div>
					<div className="text-sm text-[var(--text-strong)] mt-1">{snapshot?.paired_node_count ?? 0}</div>
				</div>
				<div className="rounded border border-[var(--border)] bg-[var(--bg)] px-3 py-2">
					<div className="text-[10px] uppercase tracking-wide text-[var(--text-muted)]">Managed Targets</div>
					<div className="text-sm text-[var(--text-strong)] mt-1">
						{snapshot?.managed_target_count ?? 0}
						{snapshot?.pinned_target_count ? (
							<span className="text-xs text-[var(--text-muted)]"> ({snapshot.pinned_target_count} pinned)</span>
						) : null}
					</div>
				</div>
				<div className="rounded border border-[var(--border)] bg-[var(--bg)] px-3 py-2">
					<div className="text-[10px] uppercase tracking-wide text-[var(--text-muted)]">Managed Keys</div>
					<div className="text-sm text-[var(--text-strong)] mt-1">
						{snapshot?.managed_key_count ?? 0}
						{snapshot?.encrypted_key_count ? (
							<span className="text-xs text-[var(--text-muted)]"> ({snapshot.encrypted_key_count} encrypted)</span>
						) : null}
					</div>
				</div>
			</div>
			{doctorError.value ? <div className="text-xs text-red-500">{doctorError.value}</div> : null}
			{doctorTest.value ? (
				<div className="rounded border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-xs">
					<div className="font-medium text-[var(--text-strong)]">
						{doctorTest.value.route_label || "Active SSH route"}
					</div>
					<div className={`${doctorTest.value.reachable ? "text-green-500" : "text-red-500"} mt-1`}>
						{doctorTest.value.reachable ? "Reachable" : "Unreachable"}
						{doctorTest.value.exit_code != null ? ` (exit ${doctorTest.value.exit_code})` : ""}
					</div>
					{doctorTest.value.failure_hint ? (
						<div className="mt-1 text-[11px] text-[var(--text-muted)]">Hint: {doctorTest.value.failure_hint}</div>
					) : null}
					{doctorTest.value.stderr ? (
						<pre className="mt-2 whitespace-pre-wrap break-all text-[11px] text-[var(--text-muted)]">
							{doctorTest.value.stderr}
						</pre>
					) : null}
				</div>
			) : null}
			<div className="flex flex-col gap-2">
				{checkList.map((check) => (
					<div key={check.title} className="rounded border border-[var(--border)] bg-[var(--bg)] px-3 py-2">
						<div className="flex items-center gap-2 flex-wrap">
							<div className="text-sm text-[var(--text-strong)]">{check.title}</div>
							<DoctorBadge level={check.level} />
						</div>
						<div className="text-xs text-[var(--text-muted)] mt-1">{check.message}</div>
						{check.hint ? <div className="text-xs text-[var(--text-muted)] mt-1">Hint: {check.hint}</div> : null}
					</div>
				))}
			</div>
		</div>
	);
}

function SshTargetCard({ node }: { node: NodeInfo }): VNode {
	const target = sshTargetValue(node) || "configured target";
	return (
		<div className="flex items-start gap-3 p-3 rounded-lg bg-[var(--surface-alt)] border border-[var(--border)]">
			<div className="w-2 h-2 rounded-full bg-sky-500 shrink-0 mt-1" title="Configured SSH target" />
			<div className="flex-1 min-w-0">
				<div className="flex items-center gap-2 flex-wrap">
					<div className="text-sm font-medium text-[var(--text-strong)] truncate">{nodeDisplayLabel(node)}</div>
					<span className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded bg-sky-500/15 text-sky-500">
						ssh
					</span>
					<span className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded bg-[var(--bg)] text-[var(--text-muted)] border border-[var(--border)]">
						configured
					</span>
				</div>
				<div className="text-xs text-[var(--text-muted)] mt-1">
					<code>{target}</code>
				</div>
				<p className="text-xs text-[var(--text-muted)] mt-2 mb-0">
					Uses your local OpenSSH configuration for remote exec. This is an execution route, not a paired WebSocket
					node, so telemetry and presence are not available here.
				</p>
			</div>
		</div>
	);
}

function ConnectedNodesList(): VNode {
	const sshTargets = nodes.value.filter(isSshTargetNode);
	const connectedNodes = nodes.value.filter((node) => !isSshTargetNode(node));
	if (connectedNodes.length === 0 && sshTargets.length === 0) {
		return (
			<div className="flex flex-col gap-4">
				<div className="text-sm text-[var(--text-muted)] py-4 text-center">
					<p>No nodes connected.</p>
				</div>
			</div>
		);
	}
	return (
		<div className="flex flex-col gap-2">
			{sshTargets.length > 0 && (
				<div className="flex flex-col gap-2">
					<div className="text-xs uppercase tracking-wide text-[var(--text-muted)]">Configured SSH Targets</div>
					{sshTargets.map((node) => (
						<SshTargetCard key={node.nodeId} node={node} />
					))}
				</div>
			)}
			{connectedNodes.length > 0 && (
				<div className="flex flex-col gap-2">
					<div className="text-xs uppercase tracking-wide text-[var(--text-muted)]">Connected Paired Nodes</div>
					{connectedNodes.map((n) => (
						<div
							key={n.nodeId}
							className="flex items-center gap-3 p-3 rounded-lg bg-[var(--surface-alt)] border border-[var(--border)]"
						>
							<div className="w-2 h-2 rounded-full bg-green-500 shrink-0" title="Connected" />
							<div className="flex-1 min-w-0">
								<div className="text-sm font-medium text-[var(--text-strong)] truncate">{nodeDisplayLabel(n)}</div>
								<div className="text-xs text-[var(--text-muted)]">
									{n.platform || "unknown"} &middot; v{n.version || "?"}
									{n.remoteIp ? <> &middot; {n.remoteIp}</> : null}
								</div>
								{n.capabilities?.length ? (
									<div className="text-xs text-[var(--text-muted)] mt-1">caps: {n.capabilities.join(", ")}</div>
								) : null}
								<NodeTelemetry telemetry={n.telemetry} />
							</div>
						</div>
					))}
				</div>
			)}
		</div>
	);
}

function PairedDevicesList(): VNode {
	if (pairedDevices.value.length === 0)
		return <div className="text-sm text-[var(--text-muted)] py-8 text-center">No paired devices.</div>;
	return (
		<div className="flex flex-col gap-2">
			{pairedDevices.value.map((d) => (
				<div
					key={d.deviceId}
					className="flex items-center gap-3 p-3 rounded-lg bg-[var(--surface-alt)] border border-[var(--border)]"
				>
					<div className="flex-1 min-w-0">
						<div className="flex items-center gap-2 flex-wrap">
							<div className="text-sm font-medium text-[var(--text-strong)] truncate">
								{d.displayName || d.deviceId}
							</div>
							{d.publicKey ? (
								<span className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded bg-green-500/15 text-green-500">
									key-verified
								</span>
							) : (
								<span className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded bg-yellow-500/15 text-yellow-500">
									token
								</span>
							)}
						</div>
						<div className="text-xs text-[var(--text-muted)]">
							{d.platform || "unknown"}
							{d.createdAt ? <> &middot; paired {d.createdAt}</> : null}
						</div>
						{d.fingerprint ? (
							<div className="text-xs font-mono text-[var(--text-muted)] mt-1 break-all">{d.fingerprint}</div>
						) : null}
					</div>
					<button
						type="button"
						className="provider-btn-danger text-xs px-2 py-1"
						onClick={() => revokeDevice(d.deviceId)}
					>
						Revoke
					</button>
				</div>
			))}
		</div>
	);
}

function PendingPairsList(): VNode {
	if (pendingPairs.value.length === 0)
		return <div className="text-sm text-[var(--text-muted)] py-8 text-center">No pending pairing requests.</div>;
	return (
		<div className="flex flex-col gap-2">
			{pendingPairs.value.map((r) => (
				<div
					key={r.id}
					className="flex items-center gap-3 p-3 rounded-lg bg-[var(--surface-alt)] border border-[var(--border)]"
				>
					<div className="flex-1 min-w-0">
						<div className="text-sm font-medium text-[var(--text-strong)] truncate">{r.displayName || r.deviceId}</div>
						<div className="text-xs text-[var(--text-muted)]">{r.platform || "unknown"}</div>
						{r.fingerprint ? (
							<div className="text-xs font-mono text-[var(--text-muted)] mt-1 break-all">{r.fingerprint}</div>
						) : null}
					</div>
					<div className="flex gap-1.5">
						<button type="button" className="provider-btn text-xs px-2 py-1" onClick={() => approvePair(r.id)}>
							Approve
						</button>
						<button type="button" className="provider-btn-secondary text-xs px-2 py-1" onClick={() => rejectPair(r.id)}>
							Reject
						</button>
					</div>
				</div>
			))}
		</div>
	);
}

function Toasts(): VNode | null {
	if (toasts.value.length === 0) return null;
	return (
		<div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
			{toasts.value.map((t) => (
				<div
					key={t.id}
					className={`px-4 py-2 rounded-lg text-sm shadow-lg ${t.type === "error" ? "bg-red-600 text-white" : "bg-green-600 text-white"}`}
				>
					{t.message}
				</div>
			))}
		</div>
	);
}

// ── Main component ──────────────────────────────────────────

function NodesPage(): VNode {
	useEffect(() => {
		refreshAll();
		const unsub = onEvent("presence", () => {
			refreshNodes();
		});
		const unsubPair = onEvent("node.pair.requested", () => {
			refreshPendingPairs();
		});
		const unsubResolved = onEvent("node.pair.resolved", () => {
			refreshAll();
		});
		const unsubDevice = onEvent("device.pair.resolved", () => {
			refreshAll();
		});
		const unsubTelemetry = onEvent("node.telemetry", (payload: unknown) => {
			const p = payload as Record<string, unknown>;
			if (!p?.nodeId) return;
			const mem = p.mem as Record<string, number> | undefined;
			nodes.value = nodes.value.map((n) => {
				if (n.nodeId !== p.nodeId) return n;
				return {
					...n,
					telemetry: {
						memTotal: mem?.total ?? n.telemetry?.memTotal,
						memAvailable: mem?.available ?? n.telemetry?.memAvailable,
						cpuCount: (p.cpuCount as number) ?? n.telemetry?.cpuCount,
						cpuUsage: (p.cpuUsage as number) ?? n.telemetry?.cpuUsage,
						uptimeSecs: (p.uptime as number) ?? n.telemetry?.uptimeSecs,
						services: (p.services as unknown[]) ?? n.telemetry?.services ?? [],
						stale: false,
					},
				};
			});
		});
		return () => {
			unsub();
			unsubPair();
			unsubResolved();
			unsubDevice();
			unsubTelemetry();
		};
	}, []);

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="max-w-form flex flex-col gap-4">
				<div>
					<div className="flex items-center gap-3 mb-1">
						<h2 className="text-lg font-medium text-[var(--text-strong)]">Nodes</h2>
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={refreshAll}
							disabled={loading.value}
						>
							{loading.value ? "Refreshing..." : "Refresh"}
						</button>
					</div>
					<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
						Nodes are remote execution targets. Paired nodes stream telemetry and capabilities back to the gateway,
						while configured SSH targets route commands through your local OpenSSH setup. The agent can choose where to
						run commands based on what is available.
					</p>
				</div>
				<RemoteExecStatusCard />
				<TabBar />
				{activeTab.value === "connected" ? (
					<>
						<ConnectNodeForm />
						<ConnectedNodesList />
					</>
				) : null}
				{activeTab.value === "paired" ? <PairedDevicesList /> : null}
				{activeTab.value === "pending" ? <PendingPairsList /> : null}
			</div>
			<Toasts />
			<ConfirmDialog />
		</div>
	);
}

// ── Mount / unmount ─────────────────────────────────────────

let containerRef: HTMLElement | null = null;

export function initNodes(container: HTMLElement): void {
	containerRef = container;
	render(<NodesPage />, container);
}

export function teardownNodes(): void {
	if (containerRef) render(null, containerRef);
	containerRef = null;
}
