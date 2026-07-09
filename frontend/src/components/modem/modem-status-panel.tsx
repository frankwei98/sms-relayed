import { Power, PowerOff, RefreshCw, RotateCcw } from "lucide-react";
import { useEffect, useState } from "react";
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
} from "#/lib/modem-api";

const POLL_INTERVAL_MS = 2000;
const POLL_LIMIT = 15;

export function ModemStatusPanel() {
	const [status, setStatus] = useState<ModemStatus | null>(null);
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState<ModemAction | null>(null);
	const [error, setError] = useState("");
	const [resetOpen, setResetOpen] = useState(false);

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

	if (loading) return <p>Loading modem status...</p>;

	return (
		<div className="mx-auto max-w-5xl space-y-6">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div>
					<h2 className="text-lg font-semibold">Modem</h2>
					<p className="text-sm text-muted-foreground">
						{status
							? `Last checked ${formatDate(status.checked_at)}`
							: "Status unavailable"}
					</p>
				</div>
				<div className="flex items-center gap-2">
					{status && <StatusBadge value={status.health.status} />}
					<Button variant="outline" onClick={refresh} disabled={!!busy}>
						<RefreshCw className="size-4" />
						Refresh
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
							label="Configured path"
							value={status.configured_modem_path}
						/>
						<Field
							label="Resolved modem"
							value={status.resolved.path ?? "not found"}
						/>
						<Field label="Enabled" value={formatBool(status.modem.enabled)} />
						<Field label="State" value={status.modem.state ?? "unknown"} />
						<Field label="SIM" value={status.modem.sim_state ?? "unknown"} />
						<Field
							label="Operator"
							value={status.modem.operator_name ?? "unknown"}
						/>
						<Field
							label="Signal"
							value={
								status.modem.signal_quality == null
									? "unknown"
									: `${status.modem.signal_quality}%`
							}
						/>
						<Field
							label="Access"
							value={status.modem.access_technologies.join(", ") || "unknown"}
						/>
						<Field
							label="Messaging"
							value={status.messaging.available ? "available" : "unavailable"}
						/>
						<Field
							label="mmcli"
							value={
								status.tool.available
									? (status.tool.version_raw ?? "available")
									: "missing"
							}
						/>
					</section>

					{(status.health.reasons.length > 0 ||
						status.diagnostics.last_error ||
						status.diagnostics.path_drift_candidate) && (
						<section className="rounded border bg-muted/30 p-4 text-sm">
							<h3 className="mb-2 font-medium">Diagnostics</h3>
							{status.health.reasons.length > 0 && (
								<p>Reasons: {status.health.reasons.join(", ")}</p>
							)}
							{status.diagnostics.path_drift_candidate && (
								<p>
									Possible new modem path:{" "}
									{status.diagnostics.path_drift_candidate}
								</p>
							)}
							{status.diagnostics.last_error && (
								<p>Error: {status.diagnostics.last_error}</p>
							)}
						</section>
					)}

					<section className="flex flex-wrap gap-2">
						<Button
							onClick={() => run("enable")}
							disabled={busy !== null || status.modem.enabled === true}
						>
							<Power className="size-4" />
							Enable
						</Button>
						<Button
							variant="outline"
							onClick={() => run("disable")}
							disabled={busy !== null || status.modem.enabled === false}
						>
							<PowerOff className="size-4" />
							Disable
						</Button>
					</section>

					<section className="space-y-2 border-t pt-4">
						<h3 className="font-medium text-destructive">Danger zone</h3>
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
								Reset modem
							</DialogTrigger>
							<DialogContent>
								<DialogHeader>
									<DialogTitle>Reset modem?</DialogTitle>
								</DialogHeader>
								<p className="text-sm text-muted-foreground">
									This can disconnect cellular service and cause the modem to
									disappear while it re-enumerates.
								</p>
								<DialogFooter>
									<Button variant="outline" onClick={() => setResetOpen(false)}>
										Cancel
									</Button>
									<Button
										variant="destructive"
										onClick={() => run("reset")}
										disabled={busy !== null}
									>
										Reset
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

function Field({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded border p-3">
			<div className="text-xs text-muted-foreground">{label}</div>
			<div className="mt-1 break-all text-sm font-medium">{value}</div>
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
