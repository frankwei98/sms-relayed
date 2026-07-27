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
	const diagnostics = [...value.reasons, ...value.warnings];

	return (
		<section className="rounded border p-4">
			<div className="flex flex-wrap items-start justify-between gap-3">
				<div>
					<h3 className="font-medium">{t("modem.smsOverIms.title")}</h3>
					<p className="mt-1 text-xs text-muted-foreground">
						{t("modem.smsOverIms.description")}
					</p>
				</div>
				<ImsStatusBadge value={value.status} technology={value.technology} />
			</div>

			<div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
				<CompactField
					label={t("modem.smsOverIms.field.configured")}
					value={formatImsEnum(value.configured)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.registration")}
					value={formatImsEnum(value.registration)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.smsService")}
					value={formatImsEnum(value.sms_service)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.technology")}
					value={formatTechnology(value.technology)}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.qmicli")}
					value={
						value.probe.available
							? (value.probe.version_raw ?? t("modem.value.available"))
							: t("modem.value.missing")
					}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.qmiDevice")}
					value={value.probe.device ?? t("modem.value.notSelected")}
				/>
				<CompactField
					label={t("modem.smsOverIms.field.evidence")}
					value={formatImsEvidence(value)}
				/>
			</div>

			{diagnostics.length > 0 && (
				<div className="mt-4 space-y-1 rounded bg-muted/40 p-3 text-xs text-muted-foreground">
					{diagnostics.map((code) => (
						<p key={code}>{imsDiagnosticMessage(code)}</p>
					))}
				</div>
			)}
		</section>
	);
}

function ImsStatusBadge({
	value,
	technology,
}: {
	value: SmsOverIms["status"];
	technology: SmsOverIms["technology"];
}) {
	const className =
		value === "available"
			? "bg-emerald-100 text-emerald-800"
			: value === "registering"
				? "bg-slate-200 text-slate-800"
				: value === "limited" || value === "not_registered"
					? "bg-amber-100 text-amber-800"
					: value === "unavailable"
						? "bg-red-100 text-red-800"
						: "bg-slate-100 text-slate-700";

	return (
		<span className={`rounded px-2 py-1 text-xs font-medium ${className}`}>
			{formatImsStatus(value, technology)}
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
) {
	if (
		status === "available" &&
		(technology === "wlan" || technology === "interworking_wlan")
	) {
		return "Available over WLAN";
	}
	return formatImsEnum(status);
}

function formatTechnology(value: SmsOverIms["technology"]) {
	if (value === "wwan") return "WWAN";
	if (value === "wlan") return "WLAN";
	if (value === "interworking_wlan") return "Interworking WLAN";
	return "Unknown";
}

function formatImsEvidence(value: SmsOverIms) {
	const source = value.evidence.some((item) => item.startsWith("qmi_imsa_"))
		? "QMI IMSA"
		: value.evidence.includes("qmi_ims_settings")
			? "QMI IMS"
			: "No runtime evidence";
	if (value.technology === "unknown" || source === "No runtime evidence") {
		return source;
	}
	return `${source} · ${formatTechnology(value.technology)}`;
}

function formatImsEnum(value: string) {
	// Maps API enum values to translated labels via the smsOverIms.enum namespace
	// The caller should wrap this in a translation context.
	const map: Record<string, string> = {
		enabled: "Enabled",
		disabled: "Disabled",
		registered: "Registered",
		registering: "Registering",
		limited: "Limited",
		not_registered: "Not Registered",
		not_available: "Not available",
		available: "Available",
		unknown: "Unknown",
		unavailable: "Unavailable",
	};
	return map[value] ?? value;
}

function imsDiagnosticMessage(code: string) {
	const messages: Record<string, string> = {
		ims_probe_not_attempted: "IMS probing was not attempted.",
		modem_not_resolved:
			"IMS probing was skipped because no modem was resolved.",
		modem_disabled: "IMS probing was skipped because the modem is disabled.",
		qmicli_missing: "qmicli is not installed or could not be executed.",
		qmicli_path_invalid: "The configured qmicli path is invalid.",
		qmicli_probe_failed: "qmicli capability detection failed.",
		ims_probe_permission_denied:
			"qmicli could not be executed due to permissions.",
		qmi_port_unavailable: "No QMI control port was reported by ModemManager.",
		qmi_port_ambiguous: "More than one QMI control port was reported.",
		qmi_proxy_unavailable: "The QMI proxy is unavailable.",
		ims_probe_timeout: "The IMS probe exceeded its time budget.",
		ims_services_query_failed: "The IMS service query failed.",
		ims_services_query_unavailable:
			"qmicli does not expose the IMS service query.",
		ims_registration_query_failed: "The IMS registration query failed.",
		ims_registration_query_unavailable:
			"qmicli does not expose the IMS registration query.",
		ims_settings_query_failed: "The IMS settings query failed.",
		ims_settings_query_unavailable:
			"qmicli does not expose the IMS settings query.",
		ims_services_output_unrecognized:
			"The IMS service response was not recognized.",
		ims_registration_output_unrecognized:
			"The IMS registration response was not recognized.",
		ims_settings_output_unrecognized:
			"The IMS settings response was not recognized.",
		ims_services_output_nonstandard:
			"The IMS service response used a nonstandard label.",
		ims_registration_output_nonstandard:
			"The IMS registration response used a nonstandard label.",
		ims_settings_output_nonstandard:
			"The IMS settings response used a nonstandard label.",
		ims_state_inconsistent:
			"The modem reported inconsistent IMS configuration and runtime state.",
	};
	return (
		messages[code] ?? "Additional IMS diagnostic information is unavailable."
	);
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
