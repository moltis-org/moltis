// ── Approve sender modal ─────────────────────────────────────
//
// Approving a sender for access and granting them host privilege are separate
// decisions, so the role is always an explicit choice here rather than a side
// effect of "Approve". Operator is pre-selected only while the account has no
// operators at all — on a fresh install the person holding the OTP code read it
// off this very UI, so they are almost certainly the owner, and making them
// hunt for their own platform ID in another settings page is pure friction.
// Once one operator exists, the next sender is a guest by default.

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect } from "preact/hooks";

import { sendRpc } from "../../../helpers";
import { Modal } from "../../../ui";
import {
	loadChannels,
	loadSenders,
	pendingApproval,
	type SenderEntry,
	selectedSenderChannel,
} from "../../ChannelsPage";

type ApprovalRole = "guest" | "operator";

// The prop is `value`, not `role`: a prop literally named `role` is read as the
// ARIA role attribute by the a11y lint, and "guest"/"operator" are not roles in
// that sense.
function RoleOption(props: {
	value: ApprovalRole;
	selected: ApprovalRole;
	title: string;
	detail: string;
	onSelect: (role: ApprovalRole) => void;
}): VNode {
	const isSelected = props.selected === props.value;
	return (
		<label
			className={`backend-card ${isSelected ? "selected" : ""} block cursor-pointer p-3`}
			data-testid={`approve-role-${props.value}`}
			data-selected={isSelected ? "true" : "false"}
		>
			<div className="flex items-start gap-2">
				<input
					type="radio"
					name="approve-role"
					className="mt-1"
					checked={isSelected}
					onChange={() => props.onSelect(props.value)}
				/>
				<div className="min-w-0">
					<div className="text-sm font-medium text-[var(--text)]">{props.title}</div>
					<div className="text-xs text-[var(--muted)]">{props.detail}</div>
				</div>
			</div>
		</label>
	);
}

export function ApproveSenderModal(): VNode | null {
	const sender: SenderEntry | null = pendingApproval.value;
	const channel = selectedSenderChannel();
	const hasOperators = (channel?.config?.operators?.length ?? 0) > 0;
	const role = useSignal<ApprovalRole>("guest");
	const error = useSignal("");
	const saving = useSignal(false);

	// This component stays mounted and renders null while closed, so the default
	// must be chosen when the dialog opens — a mount-time initializer would be
	// computed before any channel is loaded and then never revisited.
	useEffect(() => {
		if (!sender) return;
		role.value = hasOperators ? "guest" : "operator";
		error.value = "";
		saving.value = false;
	}, [sender, hasOperators, role, error, saving]);

	if (!(sender && channel)) return null;

	// Operators are matched as exact platform IDs, so the operator grant needs
	// peer_id even when the allowlist entry is a username.
	const identifier = sender.username || sender.peer_id;

	function close(): void {
		pendingApproval.value = null;
		error.value = "";
		saving.value = false;
	}

	function approve(): void {
		if (!(sender && channel)) return;
		saving.value = true;
		error.value = "";
		sendRpc("channels.senders.approve", {
			type: channel.type,
			account_id: channel.account_id,
			identifier,
			peer_id: sender.peer_id,
			role: role.value,
		})
			.then((res) => {
				const r = res as { ok?: boolean; error?: { message?: string } } | undefined;
				if (!r?.ok) {
					error.value = r?.error?.message || "Approval failed.";
					saving.value = false;
					return;
				}
				close();
				loadSenders();
				loadChannels();
			})
			.catch((e: unknown) => {
				error.value = e instanceof Error ? e.message : String(e);
				saving.value = false;
			});
	}

	return (
		<Modal show={true} onClose={close} title={`Approve ${sender.sender_name || sender.peer_id}`}>
			<div className="flex flex-col gap-3">
				<div className="text-xs text-[var(--muted)]" data-testid="approve-sender-identity">
					{sender.username ? `@${String(sender.username).replace(/^@/, "")} · ` : ""}
					{sender.peer_id}
				</div>

				<RoleOption
					value="guest"
					selected={role.value}
					title="Guest"
					detail="Can chat with the bot. No access to this machine."
					onSelect={(next) => {
						role.value = next;
					}}
				/>
				<RoleOption
					value="operator"
					selected={role.value}
					title="Operator"
					detail="Can run /sh and other commands on this machine, and read this instance's sessions and memory. This is equivalent to giving someone your terminal — grant it to yourself, or to someone you trust with the host."
					onSelect={(next) => {
						role.value = next;
					}}
				/>

				{!hasOperators && (
					<p className="text-xs text-[var(--muted)]" data-testid="approve-first-operator-hint">
						This channel has no operators yet, so nobody — including you — can run <code>/sh</code> or other privileged
						commands on it. Approve yourself as an operator to enable them.
					</p>
				)}

				{error.value && <div className="text-xs text-[var(--danger,#dc2626)]">{error.value}</div>}

				<div className="flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={close}>
						Cancel
					</button>
					<button type="button" className="provider-btn" disabled={saving.value} onClick={approve}>
						{role.value === "operator" ? "Approve as operator" : "Approve"}
					</button>
				</div>
			</div>
		</Modal>
	);
}
