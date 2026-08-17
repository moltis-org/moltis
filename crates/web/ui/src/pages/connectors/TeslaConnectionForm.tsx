import type { VNode } from "preact";
import { useState } from "preact/hooks";
import { CheckboxField, SaveButton, SelectField, StatusMessage, TextField } from "../../components/forms";
import { useTranslation } from "../../i18n";
import type { TeslaConnectorAccount, TeslaRegion } from "../../types/connector";
import { Modal, showToast } from "../../ui";
import { connectorRpc } from "./rpc";

export interface TeslaConnectionFormModalProps {
	account: TeslaConnectorAccount | null;
	onClose: () => void;
	onSaved: () => Promise<void>;
}

const TESLA_REGIONS: Array<{ value: TeslaRegion; label: string }> = [
	{ value: "north_america", label: "North America / Asia-Pacific" },
	{ value: "europe", label: "Europe, Middle East & Africa" },
	{ value: "china", label: "China" },
];

export function TeslaConnectionFormModal({ account, onClose, onSaved }: TeslaConnectionFormModalProps): VNode {
	const { t } = useTranslation("connectors");
	const [name, setName] = useState(account?.name ?? "");
	const [region, setRegion] = useState<TeslaRegion>(account?.teslaRegion ?? "north_america");
	const [clientId, setClientId] = useState(account?.teslaClientId ?? "");
	const [refreshToken, setRefreshToken] = useState("");
	const [enabled, setEnabled] = useState(account?.enabled ?? true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	async function submit(event: Event): Promise<void> {
		event.preventDefault();
		const hasIdentity = Boolean(name.trim() && clientId.trim());
		// On edit the token may be left blank to keep the stored one.
		const hasToken = Boolean(account || refreshToken.trim());
		if (!(hasIdentity && hasToken)) {
			setError(t("connections.teslaRequired"));
			return;
		}
		setSaving(true);
		setError(null);
		try {
			if (account) {
				await connectorRpc("connectors.accounts.update", {
					id: account.id,
					name: name.trim(),
					teslaRegion: region,
					teslaClientId: clientId.trim(),
					...(refreshToken.trim() ? { teslaRefreshToken: refreshToken.trim() } : {}),
					enabled,
				});
			} else {
				await connectorRpc("connectors.accounts.add", {
					kind: "tesla",
					name: name.trim(),
					teslaRegion: region,
					teslaClientId: clientId.trim(),
					teslaRefreshToken: refreshToken.trim(),
					enabled,
				});
			}
			await onSaved();
			showToast(t("connections.saved"), "success");
			onClose();
		} catch (caught: unknown) {
			setError(caught instanceof Error ? caught.message : String(caught));
		} finally {
			setSaving(false);
		}
	}

	return (
		<Modal show={true} onClose={onClose} title={t("connections.addTeslaTitle")}>
			<form onSubmit={submit} className="flex flex-col gap-1">
				<div className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3 text-xs text-[var(--muted)]">
					{t("connections.teslaSetupHelp")}
				</div>
				<TextField
					id="connector-tesla-name"
					label={t("connections.name")}
					value={name}
					onInput={setName}
					placeholder={t("connections.teslaNamePlaceholder")}
					required
				/>
				<SelectField
					id="connector-tesla-region"
					label={t("connections.teslaRegion")}
					value={region}
					onChange={(value) => setRegion(value as TeslaRegion)}
					options={TESLA_REGIONS}
				/>
				<TextField
					id="connector-tesla-client-id"
					label={t("connections.teslaClientId")}
					value={clientId}
					onInput={setClientId}
					required
				/>
				<TextField
					id="connector-tesla-refresh-token"
					label={t("connections.teslaRefreshToken")}
					type="password"
					value={refreshToken}
					onInput={setRefreshToken}
					placeholder={account ? t("connections.teslaTokenKeep") : undefined}
					required={!account}
				/>
				<div className="mb-3 text-xs text-[var(--muted)]">{t("connections.teslaStorageNote")}</div>
				<CheckboxField label={t("connections.enabled")} checked={enabled} onChange={setEnabled} />
				<StatusMessage error={error} />
				<div className="mt-3 flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						{t("cancel")}
					</button>
					<SaveButton saving={saving} label={t("connections.save")} savingLabel={t("connections.saving")} />
				</div>
			</form>
		</Modal>
	);
}
