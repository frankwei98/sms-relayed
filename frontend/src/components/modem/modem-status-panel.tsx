import type { TFunction } from "i18next";
import { Power, PowerOff, RefreshCw, RotateCcw } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PhoneNumberCopy } from "#/components/phone-number-copy";
import { Button } from "#/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "#/components/ui/dialog";
import {
	fetchModemStatus,
	type ModemAction,
	type ModemStatus,
	runModemAction,
	type SmsOverIms,
} from "#/lib/modem-api";

const POLL_INTERVAL_MS = 2000;
const POLL_LIMIT = 15;

export function ModemStatusPanel() {
	const [status, setStatus] = useState<ModemStatus | null>(null);
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState<ModemAction | null>(null);
	const [error, setError] = useState("");
	const [resetOpen, setResetOpen] = useState(false);
	const { t } = useTranslation();

	async function refresh() {
		setError("");
		try {
			setStatus(await fetchModemStatus());
		} catch (e) {
			setError((e as Error).message);
		} finally {
			setLoading(false);
		}
	}

	useEffect(() => {
		setError("");
		fetchModemStatus()
			.then(setStatus)
			.catch((e) => setError((e as Error).message))
			.finally(() => setLoading(false));
	}, []);

	async function run(action: ModemAction) {
		setBusy(action);
		setError("");
		try {
			await runModemAction(action);
			if (action === "reset") {
				setResetOpen(false);
			}
			await pollStatus();
		} catch (e) {
			setError((e as Error).message);
		} finally {
			setBusy(null);
		}
	}

	async function pollStatus() {
		for (let i = 0; i < POLL_LIMIT; i++) {
			await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
			const next = await fetchModemStatus();
			setStatus(next);
			if (
				(next.resolved.present && next.health.status !== "unknown") ||
				next.diagnostics.path_drift_candidate
			) {
				return;
			}
		}
	}

	if (loading) return <p>{t("modem.loading")}</p>;

	return (
		<div className="mx-auto max-w-5xl space-y-6">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div>
					<h2 className="text-lg font-semibold">{t("modem.title")}</h2>
					<p className="text-sm text-muted-foreground">
						{status
							? t("modem.lastChecked", { time: formatDate(status.checked_at) })
							: t("modem.statusUnavailable")}
					</p>
				</div>
				<div className="flex items-center gap-2">
					{status && <StatusBadge value={status.health.status} />}
					<Button variant="outline" onClick={refresh} disabled={!!busy}>
						<RefreshCw className="size-4" />
						{t("common.refresh")}
					</Button>
				</div>
			</div>

			{error && (
				<div className="rounded border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
					{error}
				</div>
			)}

			{status && (
				<>
					<section className="grid gap-3 md:grid-cols-2">
						<Field
							label={t("modem.field.configuredPath")}
							value={status.configured_modem_path}
						/>
						<Field
							label={t("modem.field.resolvedModem")}
							value={status.resolved.path ?? t("modem.value.notFound")}
						/>
						<Field
							label={t("modem.field.enabled")}
							value={formatBool(status.modem.enabled)}
						/>
						<Field
							label={t("modem.field.state")}
							value={status.modem.state ?? t("modem.value.unknown")}
						/>
						<Field
							label={t("modem.field.sim")}
							value={status.modem.sim_state ?? t("modem.value.unknown")}
						/>
						<Field
							label={t("modem.field.phoneNumber")}
							value={status.modem.own_number ?? t("modem.value.notReported")}
							action={
								status.modem.own_number ? (
									<PhoneNumberCopy phoneNumber={status.modem.own_number} />
								) : null
							}
						/>
						<Field
							label={t("modem.field.operator")}
							value={status.modem.operator_name ?? t("modem.value.unknown")}
						/>
						<Field
							label={t("modem.field.signal")}
							value={
								status.modem.signal_quality == null
									? t("modem.value.unknown")
									: `${status.modem.signal_quality}%`
							}
						/>
						<Field
							label={t("modem.field.access")}
							value={
								status.modem.access_technologies.join(", ") ||
								t("modem.value.unknown")
							}
						/>
						<Field
							label={t("modem.field.messaging")}
							value={
								status.messaging.available
									? t("modem.value.available")
									: t("modem.value.unavailable")
							}
						/>
						<Field
							label={t("modem.field.mmcli")}
							value={
								status.tool.available
									? (status.tool.version_raw ?? t("modem.value.available"))
									: t("modem.value.missing")
							}
						/>
					</section>

					<SmsOverImsCard value={status.sms_over_ims} />

					{(status.health.reasons.length > 0 ||
						status.diagnostics.last_error ||
						status.diagnostics.path_drift_candidate) && (
						<section className="rounded border bg-muted/30 p-4 text-sm">
							<h3 className="mb-2 font-medium">
								{t("modem.diagnostics.title")}
							</h3>
							{status.health.reasons.length > 0 && (
								<p>
									{t("modem.diagnostics.reasons", {
										reasons: status.health.reasons.join(", "),
									})}
								</p>
							)}
							{status.diagnostics.path_drift_candidate && (
								<p>
									{t("modem.diagnostics.possibleNewPath", {
										path: status.diagnostics.path_drift_candidate,
									})}
								</p>
							)}
							{status.diagnostics.last_error && (
								<p>
									{t("modem.diagnostics.error", {
										error: status.diagnostics.last_error,
									})}
								</p>
							)}
						</section>
					)}

					<section className="flex flex-wrap gap-2">
						<Button
							onClick={() => run("enable")}
							disabled={busy !== null || status.modem.enabled === true}
						>
							<Power className="size-4" />
							{t("modem.actions.enable")}
						</Button>
						<Button
							variant="outline"
							onClick={() => run("disable")}
							disabled={busy !== null || status.modem.enabled === false}
						>
							<PowerOff className="size-4" />
							{t("modem.actions.disable")}
						</Button>
					</section>

					<section className="space-y-2 border-t pt-4">
						<h3 className="font-medium text-destructive">
							{t("modem.dangerZone.title")}
						</h3>
						<Dialog open={resetOpen} onOpenChange={setResetOpen}>
							<DialogTrigger
								render={
									<Button
										type="button"
										variant="destructive"
										disabled={busy !== null}
									/>
								}
							>
								<RotateCcw className="size-4" />
								{t("modem.dangerZone.reset")}
							</DialogTrigger>
							<DialogContent>
								<DialogHeader>
									<DialogTitle>{t("modem.dangerZone.dialogTitle")}</DialogTitle>
								</DialogHeader>
								<p className="text-sm text-muted-foreground">
									{t("modem.dangerZone.dialogDescription")}
								</p>
								<DialogFooter>
									<Button variant="outline" onClick={() => setResetOpen(false)}>
										{t("modem.dangerZone.cancel")}
									</Button>
									<Button
										variant="destructive"
										onClick={() => run("reset")}
										disabled={busy !== null}
									>
										{t("modem.dangerZone.confirmReset")}
									</Button>
								</DialogFooter>
							</DialogContent>
						</Dialog>
					</section>
				</>
			)}
		</div>
	);
}

function SmsOverImsCard({ value }: { value: SmsOverIms }) {
	const { t } = useTranslation();
	const diagnostics = [...new Set([...value.reasons, ...value.warnings])];
	const booleanValue = (candidate: boolean | null) =>
		candidate === null
			? t("modem.value.unknown")
			: candidate
				? t("modem.value.yes")
				: t("modem.value.no");

	return (
		<section className="rounded border p-4">
			<div className="flex flex-wrap items-start justify-between gap-3">
				<div>
					<h3 className="font-medium">{t("modem.smsOverIms.title")}</h3>
					<p className="mt-1 text-xs text-muted-foreground">
						{t("modem.smsOverIms.description")}
					</p>
				</div>
				<VoiceImsStatusBadge value={value.voice_over_ims} />
			</div>

			<div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
				<CompactField
					label={t("modem.smsOverIms.field.lteVoiceSupport")}
					value={booleanValue(value.lte_voice_support)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.imsVoiceSupport")}
					value={booleanValue(value.ims_voice_support)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.volteConfigured")}
					value={formatImsEnum(value.volte_configured, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.vowifiConfigured")}
					value={formatImsEnum(value.vowifi_configured, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.registration")}
					value={formatImsEnum(value.registration, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.voiceService")}
					value={formatImsEnum(value.voice_service, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.voiceTechnology")}
					value={formatTechnology(value.voice_technology, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.smsConfigured")}
					value={formatImsEnum(value.configured, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.smsStatus")}
					value={formatImsStatus(value.status, value.technology, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.smsService")}
					value={formatImsEnum(value.sms_service, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.technology")}
					value={formatTechnology(value.technology, t)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.detector")}
					value={
						value.probe.available
							? (value.probe.version_raw ?? value.probe.tool)
							: t("modem.value.missing")
					}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.qmiDevice")}
					value={value.probe.device ?? t("modem.value.notSelected")}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.evidence")}
					value={formatImsEvidence(value, t)}
				/>
			</div>

			{diagnostics.length > 0 && (
				<div className="mt-4 space-y-1 rounded bg-muted/40 p-3 text-xs text-muted-foreground">
					{diagnostics.map((code) => (
						<p key={code}>{imsDiagnosticMessage(code, t)}</p>
					))}
				</div>
			)}
		</section>
	);
}

function VoiceImsStatusBadge({
	value,
}: {
	value: SmsOverIms["voice_over_ims"];
}) {
	const { t } = useTranslation();
	const className =
		value === "volte" || value === "vowifi"
			? "bg-emerald-100 text-emerald-800"
			: value === "registering"
				? "bg-slate-200 text-slate-800"
				: value === "limited" || value === "not_registered"
					? "bg-amber-100 text-amber-800"
					: value === "unavailable"
						? "bg-red-100 text-red-800"
						: "bg-slate-100 text-slate-700";
	const label =
		value === "volte"
			? t("modem.smsOverIms.voiceStatus.volte")
			: value === "vowifi"
				? t("modem.smsOverIms.voiceStatus.vowifi")
				: formatImsEnum(value, t);

	return (
		<span className={`rounded px-2 py-1 text-xs font-medium ${className}`}>
			{label}
		</span>
	);
}

function CompactField({ label, value }: { label: string; value: string }) {
	return (
		<div>
			<div className="text-xs text-muted-foreground">{label}</div>
			<div className="mt-1 break-all text-sm font-medium">{value}</div>
		</div>
	);
}

function formatImsStatus(
	status: SmsOverIms["status"],
	technology: SmsOverIms["technology"],
	t: TFunction,
) {
	if (
		status === "available" &&
		(technology === "wlan" || technology === "interworking_wlan")
	) {
		return t("modem.smsOverIms.availableOverWlan");
	}
	return formatImsEnum(status, t);
}

function formatTechnology(value: SmsOverIms["technology"], t: TFunction) {
	if (value === "wwan") return t("modem.smsOverIms.technology.wwan");
	if (value === "wlan") return t("modem.smsOverIms.technology.wlan");
	if (value === "interworking_wlan")
		return t("modem.smsOverIms.technology.interworkingWlan");
	return t("modem.smsOverIms.technology.unknown");
}

function formatImsEvidence(value: SmsOverIms, t: TFunction) {
	if (value.evidence.some((item) => item.startsWith("qmi_imsa_"))) {
		const parts = [t("modem.smsOverIms.evidence.qmiImsa")];
		if (
			value.evidence.includes("qmi_imsa_voice") &&
			value.voice_technology !== "unknown"
		) {
			parts.push(
				`${t("modem.smsOverIms.evidence.voice")} ${formatTechnology(value.voice_technology, t)}`,
			);
		}
		if (
			value.evidence.includes("qmi_imsa_services") &&
			value.technology !== "unknown"
		) {
			parts.push(
				`${t("modem.smsOverIms.evidence.sms")} ${formatTechnology(value.technology, t)}`,
			);
		}
		return parts.join(" · ");
	}
	if (value.evidence.includes("qmi_ims_settings")) {
		return t("modem.smsOverIms.evidence.qmiIms");
	}
	if (value.evidence.includes("qmi_nas_ims_voice_support")) {
		return t("modem.smsOverIms.evidence.qmiNas");
	}
	return t("modem.smsOverIms.evidence.noEvidence");
}

type ImsEnumValue =
	| SmsOverIms["status"]
	| Exclude<SmsOverIms["voice_over_ims"], "volte" | "vowifi">
	| SmsOverIms["registration"]
	| SmsOverIms["voice_service"]
	| SmsOverIms["sms_service"]
	| SmsOverIms["configured"]
	| SmsOverIms["volte_configured"]
	| SmsOverIms["vowifi_configured"];

const imsEnumKeys = {
	enabled: "modem.smsOverIms.enum.enabled",
	disabled: "modem.smsOverIms.enum.disabled",
	registered: "modem.smsOverIms.enum.registered",
	registering: "modem.smsOverIms.enum.registering",
	limited: "modem.smsOverIms.enum.limited",
	not_registered: "modem.smsOverIms.enum.notRegistered",
	available: "modem.smsOverIms.enum.available",
	unknown: "modem.smsOverIms.enum.unknown",
	unavailable: "modem.smsOverIms.enum.unavailable",
} as const satisfies Record<ImsEnumValue, string>;

function formatImsEnum(value: ImsEnumValue, t: TFunction) {
	return t(imsEnumKeys[value]);
}

function imsDiagnosticMessage(code: string, t: TFunction) {
	const keys = {
		ims_probe_not_attempted: "modem.imsDiagnostics.imsProbeNotAttempted",
		modem_not_resolved: "modem.imsDiagnostics.modemNotResolved",
		modem_disabled: "modem.imsDiagnostics.modemDisabled",
		ims_probe_permission_denied:
			"modem.imsDiagnostics.imsProbePermissionDenied",
		qmi_port_unavailable: "modem.imsDiagnostics.qmiPortUnavailable",
		qmi_port_ambiguous: "modem.imsDiagnostics.qmiPortAmbiguous",
		qmi_proxy_unavailable: "modem.imsDiagnostics.qmiProxyUnavailable",
		native_qmi_probe_failed: "modem.imsDiagnostics.nativeQmiProbeFailed",
		ims_probe_timeout: "modem.imsDiagnostics.imsProbeTimeout",
		ims_voice_support_query_failed:
			"modem.imsDiagnostics.imsVoiceSupportQueryFailed",
		ims_voice_support_query_unavailable:
			"modem.imsDiagnostics.imsVoiceSupportQueryUnavailable",
		ims_services_query_failed: "modem.imsDiagnostics.imsServicesQueryFailed",
		ims_services_query_unavailable:
			"modem.imsDiagnostics.imsServicesQueryUnavailable",
		ims_registration_query_failed:
			"modem.imsDiagnostics.imsRegistrationQueryFailed",
		ims_registration_query_unavailable:
			"modem.imsDiagnostics.imsRegistrationQueryUnavailable",
		ims_settings_query_failed: "modem.imsDiagnostics.imsSettingsQueryFailed",
		ims_settings_query_unavailable:
			"modem.imsDiagnostics.imsSettingsQueryUnavailable",
		ims_voice_output_unrecognized:
			"modem.imsDiagnostics.imsVoiceOutputUnrecognized",
		ims_services_output_unrecognized:
			"modem.imsDiagnostics.imsServicesOutputUnrecognized",
		ims_registration_output_unrecognized:
			"modem.imsDiagnostics.imsRegistrationOutputUnrecognized",
		ims_settings_output_unrecognized:
			"modem.imsDiagnostics.imsSettingsOutputUnrecognized",
		ims_volte_setting_unavailable:
			"modem.imsDiagnostics.imsVolteSettingUnavailable",
		ims_vowifi_setting_unavailable:
			"modem.imsDiagnostics.imsVowifiSettingUnavailable",
		ims_sms_setting_unavailable:
			"modem.imsDiagnostics.imsSmsSettingUnavailable",
		ims_services_output_nonstandard:
			"modem.imsDiagnostics.imsServicesOutputNonstandard",
		ims_registration_output_nonstandard:
			"modem.imsDiagnostics.imsRegistrationOutputNonstandard",
		ims_settings_output_nonstandard:
			"modem.imsDiagnostics.imsSettingsOutputNonstandard",
		ims_state_inconsistent: "modem.imsDiagnostics.imsStateInconsistent",
	} as const;
	return code in keys
		? t(keys[code as keyof typeof keys])
		: `${t("modem.imsDiagnostics.fallback")} (${code})`;
}

function StatusBadge({ value }: { value: ModemStatus["health"]["status"] }) {
	const className =
		value === "ok"
			? "bg-emerald-100 text-emerald-800"
			: value === "degraded"
				? "bg-amber-100 text-amber-800"
				: value === "error"
					? "bg-red-100 text-red-800"
					: "bg-slate-100 text-slate-700";
	return (
		<span className={`rounded px-2 py-1 text-xs font-medium ${className}`}>
			{value.toUpperCase()}
		</span>
	);
}

function Field({
	label,
	value,
	action,
}: {
	label: string;
	value: string;
	action?: ReactNode;
}) {
	return (
		<div className="rounded border p-3">
			<div className="text-xs text-muted-foreground">{label}</div>
			<div className="mt-1 flex items-center justify-between gap-2">
				<div className="break-all text-sm font-medium">{value}</div>
				{action}
			</div>
		</div>
	);
}

function formatBool(value: boolean | null) {
	if (value === true) return "yes";
	if (value === false) return "no";
	return "unknown";
}

function formatDate(value: string) {
	return new Date(value).toLocaleString();
}
