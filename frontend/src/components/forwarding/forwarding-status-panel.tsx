import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Badge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "#/components/ui/table";
import { apiFetch, type ForwardingResponse } from "#/lib/api";

export function ForwardingStatusPanel() {
	const [data, setData] = useState<ForwardingResponse | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState("");
	const [refreshing, setRefreshing] = useState(false);
	const generationRef = useRef(0);

	const refresh = useCallback(async () => {
		const gen = ++generationRef.current;
		setError("");
		try {
			const result = await apiFetch<ForwardingResponse>(
				"/api/forwarding/attempts",
			);
			if (gen === generationRef.current) {
				setData(result);
			}
		} catch (e) {
			if (gen === generationRef.current) {
				setError((e as Error).message);
			}
		} finally {
			if (gen === generationRef.current) {
				setLoading(false);
				setRefreshing(false);
			}
		}
	}, []);

	useEffect(() => {
		refresh();
	}, [refresh]);

	async function handleRefresh() {
		setRefreshing(true);
		await refresh();
	}

	if (loading) return <p>Loading forwarding status...</p>;

	return (
		<div className="mx-auto max-w-5xl space-y-6">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div>
					<h2 className="text-lg font-semibold">Forwarding</h2>
					{data && (
						<p className="text-sm text-muted-foreground">
							Last updated {new Date(data.generated_at).toLocaleString()}
						</p>
					)}
				</div>
				<Button variant="outline" onClick={handleRefresh} disabled={refreshing}>
					<RefreshCw className={`size-4 ${refreshing ? "animate-spin" : ""}`} />
					Refresh
				</Button>
			</div>

			{error && (
				<div className="rounded border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
					{error}
				</div>
			)}

			{data && data.profiles.length === 0 && (
				<p className="text-sm text-muted-foreground">
					No forwarding profiles configured.
				</p>
			)}

			{data?.profiles.map((profile) => (
				<section key={profile.profile_key} className="rounded border p-4">
					<div className="mb-3 flex items-center gap-2">
						<span className="font-mono text-sm font-medium">
							{profile.profile_key}
						</span>
						{profile.enabled ? (
							<Badge variant="default">Enabled</Badge>
						) : (
							<Badge variant="outline">Disabled</Badge>
						)}
						{profile.samples.length > 0 && (
							<OutcomeBadge outcome={profile.samples[0].outcome} />
						)}
					</div>

					{profile.samples.length === 0 ? (
						<p className="text-sm text-muted-foreground">
							No forwarding attempts yet.
						</p>
					) : (
						<Table>
							<TableHeader>
								<TableRow>
									<TableHead>Attempt</TableHead>
									<TableHead>Completed</TableHead>
									<TableHead>Timing</TableHead>
									<TableHead>Outcome</TableHead>
									<TableHead>Error</TableHead>
								</TableRow>
							</TableHeader>
							<TableBody>
								{profile.samples.map((s) => (
									<TableRow key={`${s.attempt_number}-${s.completed_at}`}>
										<TableCell className="font-mono text-xs">
											{s.attempt_number}
											{s.is_retry && (
												<Badge variant="outline" className="ml-1">
													Retry
												</Badge>
											)}
										</TableCell>
										<TableCell className="text-xs">
											{new Date(s.completed_at).toLocaleString()}
										</TableCell>
										<TableCell className="font-mono text-xs">
											{formatAttemptTiming(s.dispatch_delay_ms, s.latency_ms)}
										</TableCell>
										<TableCell>
											<OutcomeBadge outcome={s.outcome} />
										</TableCell>
										<TableCell className="font-mono text-xs text-muted-foreground">
											{s.error_code ?? "—"}
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
					)}
				</section>
			))}
		</div>
	);
}

function OutcomeBadge({ outcome }: { outcome: string }) {
	const className =
		outcome === "success"
			? "bg-emerald-100 text-emerald-800"
			: outcome === "transient_failure"
				? "bg-amber-100 text-amber-800"
				: "bg-red-100 text-red-800";
	const label =
		outcome === "success"
			? "Success"
			: outcome === "transient_failure"
				? "Transient"
				: "Failed";
	return (
		<span className={`rounded px-2 py-1 text-xs font-medium ${className}`}>
			{label}
		</span>
	);
}

function formatLatency(ms: number): string {
	if (ms < 1000) {
		return `${ms}ms`;
	}
	return `${(ms / 1000).toFixed(1)}s`;
}

function formatAttemptTiming(
	dispatchDelayMs: number | null,
	requestLatencyMs: number,
): string {
	const dispatch =
		dispatchDelayMs === null ? "—" : formatLatency(dispatchDelayMs);
	return `Dispatch ${dispatch} · Request ${formatLatency(requestLatencyMs)}`;
}
