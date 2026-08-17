// ── Notifications section ─────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { SectionHeading, StatusMessage, SubHeading } from "../../components/forms";
import { onEvent } from "../../events";
import * as push from "../../push";
import { isStandalone } from "../../pwa";
import { rerender } from "./_shared";

interface PushSubscription {
	endpoint: string;
	device?: string;
	ip?: string;
	created_at?: string;
}

interface PushServerStatus {
	subscription_count?: number;
	subscriptions?: PushSubscription[];
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Notifications section handles multiple states and conditions
export function NotificationsSection(): VNode {
	const [supported, setSupported] = useState(false);
	const [permission, setPermission] = useState("default");
	const [subscribed, setSubscribed] = useState(false);
	const [isLoading, setIsLoading] = useState(true);
	const [toggling, setToggling] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [serverStatus, setServerStatus] = useState<PushServerStatus | null>(null);
	const [testing, setTesting] = useState(false);
	const [testResult, setTestResult] = useState<string | null>(null);

	async function checkStatus(): Promise<void> {
		setIsLoading(true);
		rerender();

		const pushSupported = push.isPushSupported();
		setSupported(pushSupported);

		if (pushSupported) {
			setPermission(push.getPermissionState());
			await push.initPushState();
			setSubscribed(push.isSubscribed());

			const status = await push.getPushStatus();
			setServerStatus(status as PushServerStatus);
		}

		setIsLoading(false);
		rerender();
	}

	async function refreshStatus(): Promise<void> {
		const status = await push.getPushStatus();
		setServerStatus(status as PushServerStatus);
		rerender();
	}

	async function onRemoveSubscription(endpoint: string): Promise<void> {
		const result = await push.removeSubscription(endpoint);
		if (!result.success) {
			setError(result.error || "Failed to remove subscription");
			rerender();
		}
	}

	useEffect(() => {
		checkStatus();
		const off = onEvent("push.subscriptions", () => {
			refreshStatus();
		});
		return off;
	}, []);

	async function onToggle(): Promise<void> {
		setError(null);
		setToggling(true);
		rerender();

		const result = subscribed ? await push.unsubscribeFromPush() : await push.subscribeToPush();

		if (result.success) {
			setSubscribed(!subscribed);
			if (!subscribed) setPermission("granted");
		} else {
			setError(result.error || (subscribed ? "Failed to unsubscribe" : "Failed to subscribe"));
		}

		setToggling(false);
		rerender();
	}

	async function onSendTest(): Promise<void> {
		setTesting(true);
		setTestResult(null);
		rerender();

		const result = await push.sendTestNotification();
		if (result.success) {
			setTestResult(
				result.sent === 0
					? "No devices accepted the notification — check that a device is still subscribed."
					: `Sent to ${result.sent} device${result.sent === 1 ? "" : "s"}.`,
			);
		} else {
			const accepted = result.sent ? ` ${result.sent} accepted.` : "";
			setTestResult(`${result.error || "Failed to send test notification"}.${accepted}`);
		}

		// Sending prunes endpoints rejected as expired by the push service.
		await refreshStatus();
		setTesting(false);
		rerender();
	}

	if (isLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Notifications" />
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	if (!supported) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Notifications" />
				<div className="max-w-[600px] rounded-md border border-[var(--border)] bg-[var(--surface)] px-4 py-3">
					<p className="m-0 text-sm text-[var(--text)]">Push notifications are not supported in this browser.</p>
					<p className="mt-2 mb-0 text-xs text-[var(--muted)]">
						Try using Safari, Chrome, or Firefox on a device that supports web push.
					</p>
				</div>
			</div>
		);
	}

	if (serverStatus === null) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Notifications" />
				<div className="max-w-[600px] rounded-md border border-[var(--border)] bg-[var(--surface)] px-4 py-3">
					<p className="m-0 text-sm text-[var(--text)]">Push notifications are not configured on the server.</p>
					<p className="mt-2 mb-0 text-xs text-[var(--muted)]">
						The server was built without the <code className="font-mono text-xs">push-notifications</code> feature.
					</p>
				</div>
			</div>
		);
	}

	const standalone = isStandalone();
	const needsInstall = !standalone && /Safari/.test(navigator.userAgent) && !/Chrome/.test(navigator.userAgent);

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="Notifications" />
			<p className="m-0 max-w-[600px] text-xs leading-relaxed text-[var(--muted)]">
				Receive push notifications when the agent completes a task or needs your attention.
			</p>

			<div className="max-w-[600px]">
				<div className="provider-item mb-0">
					<div className="min-w-0 flex-1">
						<div className="provider-item-name text-sm">Push Notifications</div>
						<div className="mt-0.5 text-xs text-[var(--muted)]">
							{needsInstall
								? "Add this app to your Dock to enable notifications."
								: subscribed
									? "You will receive notifications on this device."
									: permission === "denied"
										? "Notifications are blocked. Enable them in browser settings."
										: "Enable to receive notifications on this device."}
						</div>
					</div>
					<button
						type="button"
						className={`provider-btn ${subscribed ? "provider-btn-danger" : ""}`}
						onClick={onToggle}
						disabled={toggling || permission === "denied" || needsInstall}
						aria-busy={toggling}
						aria-label={subscribed ? "Disable push notifications" : "Enable push notifications"}
					>
						{toggling ? (subscribed ? "Disabling\u2026" : "Enabling\u2026") : subscribed ? "Disable" : "Enable"}
					</button>
				</div>
				<div role="alert" aria-live="assertive">
					<StatusMessage error={error} className="text-xs mt-2" />
				</div>

				{(serverStatus?.subscription_count || 0) > 0 ? (
					<div className="provider-item mt-1.5 mb-0">
						<div className="min-w-0 flex-1">
							<div className="provider-item-name text-sm">Test Notification</div>
							<div className="mt-0.5 text-xs text-[var(--muted)]">
								Send a notification to every subscribed device to verify delivery.
							</div>
						</div>
						<button
							type="button"
							className="provider-btn provider-btn-secondary"
							onClick={onSendTest}
							disabled={testing}
							aria-busy={testing}
							aria-label="Send test notification"
						>
							{testing ? "Sending\u2026" : "Send"}
						</button>
					</div>
				) : null}
				<div className="mt-2 text-xs text-[var(--muted)]" role="status" aria-live="polite" aria-atomic="true">
					{testResult}
				</div>
			</div>

			<p className="m-0 max-w-[600px] text-xs leading-relaxed text-[var(--muted)]">
				Notifications are grouped per chat, so a busy conversation produces one notification rather than a stack. The
				device you are actively reading a chat on is skipped — other devices still get notified.
			</p>

			{needsInstall ? (
				<div className="max-w-[600px] rounded-md border border-[var(--border)] bg-[var(--surface)] px-4 py-3">
					<p className="m-0 text-sm font-medium text-[var(--text)]">Installation required</p>
					<p className="mt-2 mb-0 text-xs text-[var(--muted)]">
						On Safari, push notifications are only available for installed apps. Add moltis to your Dock using{" "}
						<strong>File {"\u2192"} Add to Dock</strong> (or Share {"\u2192"} Add to Dock on iOS), then open it from
						there.
					</p>
				</div>
			) : null}

			{permission === "denied" && !needsInstall ? (
				<div className="max-w-[600px] rounded-md border border-[var(--error)] bg-[var(--surface)] px-4 py-3">
					<p className="m-0 text-sm font-medium text-[var(--error)]">Notifications are blocked</p>
					<p className="mt-2 mb-0 text-xs text-[var(--muted)]">
						You previously blocked notifications for this site. To enable them, you'll need to update your browser's
						site settings and allow notifications for this origin.
					</p>
				</div>
			) : null}

			<div className="mt-2 max-w-[600px] border-t border-[var(--border)] pt-4">
				<SubHeading title={`Subscribed Devices (${serverStatus?.subscription_count || 0})`} />
				{(serverStatus?.subscriptions?.length || 0) > 0 ? (
					<div className="flex flex-col gap-1.5">
						{serverStatus?.subscriptions?.map((sub) => (
							<div className="provider-item mb-0" key={sub.endpoint}>
								<div className="min-w-0 flex-1">
									<div className="provider-item-name text-[.85rem]">{sub.device}</div>
									<div className="mt-0.5 flex flex-wrap gap-3 text-[.7rem] text-[var(--muted)]">
										{sub.ip ? <span className="font-mono">{sub.ip}</span> : null}
										<time dateTime={sub.created_at}>{new Date(sub.created_at || "").toLocaleDateString()}</time>
									</div>
								</div>
								<button
									type="button"
									className="provider-btn provider-btn-danger"
									onClick={() => onRemoveSubscription(sub.endpoint)}
								>
									Remove
								</button>
							</div>
						))}
					</div>
				) : (
					<div className="py-1 text-xs text-[var(--muted)]">No devices subscribed yet.</div>
				)}
			</div>
		</div>
	);
}
