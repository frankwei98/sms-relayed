import {
	Archive,
	CheckCircle2,
	ChevronLeft,
	CircleAlert,
	CircleDashed,
	CircleOff,
	CircleX,
	Clock3,
	History,
	LayoutDashboard,
	type LucideIcon,
	Power,
	RefreshCw,
	ServerCog,
	SlidersHorizontal,
	TriangleAlert,
	X,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
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
import { WorkspaceLayout } from "#/components/ui/workspace-layout";
import {
	apiFetch,
	type ForwardAttemptOutcome,
	type ForwardAttemptSample,
	type ForwardingResponse,
	type ProfileStatus,
} from "#/lib/api";
import { cn } from "#/lib/utils";

type ForwardingStatusPanelProps = {
	selectedProfile?: string;
	onSelectProfile: (profile?: string) => void;
};

type RefreshAction = {
	refreshing: boolean;
	onRefresh: () => void;
};

export function ForwardingStatusPanel({
	selectedProfile,
	onSelectProfile,
}: ForwardingStatusPanelProps) {
	const [data, setData] = useState<ForwardingResponse | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState("");
	const [refreshing, setRefreshing] = useState(false);
	const [mobileNavigationOpen, setMobileNavigationOpen] = useState(
		() => selectedProfile === undefined,
	);
	const generationRef = useRef(0);
	const previousSelectionRef = useRef(selectedProfile);
	const navigationId = useId();

	const refresh = useCallback(async () => {
		const generation = ++generationRef.current;
		setError("");
		try {
			const result = await apiFetch<ForwardingResponse>(
				"/api/forwarding/attempts",
			);
			if (generation === generationRef.current) {
				setData(result);
			}
		} catch (refreshError) {
			if (generation === generationRef.current) {
				setError(errorMessage(refreshError));
			}
		} finally {
			if (generation === generationRef.current) {
				setLoading(false);
				setRefreshing(false);
			}
		}
	}, []);

	useEffect(() => {
		void refresh();
		return () => {
			generationRef.current += 1;
		};
	}, [refresh]);

	useEffect(() => {
		if (previousSelectionRef.current === selectedProfile) return;
		previousSelectionRef.current = selectedProfile;
		setMobileNavigationOpen(false);
	}, [selectedProfile]);

	async function handleRefresh() {
		if (refreshing) return;
		setRefreshing(true);
		await refresh();
	}

	function handleSelectProfile(profile?: string) {
		setMobileNavigationOpen(false);
		onSelectProfile(profile);
	}

	const refreshAction: RefreshAction = {
		refreshing,
		onRefresh: () => void handleRefresh(),
	};

	if (loading && !data) {
		return <InitialLoadingState />;
	}

	if (!data) {
		return (
			<InitialErrorState
				error={error}
				refreshing={refreshing}
				onRefresh={refreshAction.onRefresh}
			/>
		);
	}

	const configuredProfiles: ProfileStatus[] = [];
	const historicalProfiles: ProfileStatus[] = [];
	for (const profile of data.profiles) {
		if (profile.configured) configuredProfiles.push(profile);
		else historicalProfiles.push(profile);
	}

	const activeProfile =
		selectedProfile === undefined
			? undefined
			: data.profiles.find(
					(profile) => profile.profile_key === selectedProfile,
				);

	return (
		<div className="h-full min-h-0" aria-busy={refreshing}>
			<WorkspaceLayout
				navigationLabel="Forwarding profiles"
				mobileNavigationOpen={mobileNavigationOpen}
				navigation={
					<ForwardingNavigation
						configuredProfiles={configuredProfiles}
						historicalProfiles={historicalProfiles}
						selectedProfile={selectedProfile}
						generatedAt={data.generated_at}
						sampleLimit={data.sample_limit}
						navigationId={navigationId}
						navigationOpen={mobileNavigationOpen}
						error={error}
						onSelectProfile={handleSelectProfile}
						onClose={() => setMobileNavigationOpen(false)}
						{...refreshAction}
					/>
				}
			>
				<div className="flex h-full min-h-0 flex-col">
					<DetailHeader
						selectedProfile={selectedProfile}
						activeProfile={activeProfile}
						generatedAt={data.generated_at}
						navigationId={navigationId}
						navigationOpen={mobileNavigationOpen}
						onOpenNavigation={() => setMobileNavigationOpen(true)}
						{...refreshAction}
					/>
					{error ? <RefreshErrorBanner error={error} /> : null}
					<div className="min-h-0 flex-1 overflow-y-auto">
						{selectedProfile === undefined ? (
							<ForwardingOverview
								profiles={data.profiles}
								sampleLimit={data.sample_limit}
								onSelectProfile={handleSelectProfile}
							/>
						) : activeProfile ? (
							<ProfileDetail
								profile={activeProfile}
								sampleLimit={data.sample_limit}
							/>
						) : (
							<UnavailableProfile
								profileKey={selectedProfile}
								onOverview={() => handleSelectProfile(undefined)}
							/>
						)}
					</div>
				</div>
			</WorkspaceLayout>
			<output className="sr-only" aria-live="polite" aria-atomic="true">
				{refreshing
					? "Refreshing forwarding status."
					: `Forwarding snapshot generated ${formatTimestamp(data.generated_at)}.`}
			</output>
		</div>
	);
}

function InitialLoadingState() {
	return (
		<div
			className="grid h-full min-h-64 place-items-center bg-background px-6"
			aria-busy="true"
		>
			<output
				className="flex items-center gap-3 text-sm text-muted-foreground"
				aria-live="polite"
			>
				<RefreshCw className="size-4 animate-spin" aria-hidden="true" />
				<span>Loading forwarding status...</span>
			</output>
		</div>
	);
}

function InitialErrorState({
	error,
	refreshing,
	onRefresh,
}: { error: string } & RefreshAction) {
	return (
		<div className="grid h-full min-h-64 place-items-center bg-background px-5">
			<div className="w-full max-w-md rounded-xl border bg-card p-6 shadow-sm">
				<div className="mb-4 grid size-10 place-items-center rounded-full bg-destructive/10 text-destructive dark:bg-destructive/20">
					<CircleAlert className="size-5" aria-hidden="true" />
				</div>
				<div role="alert">
					<h2 className="text-base font-semibold">
						Unable to load forwarding status
					</h2>
					<p className="mt-1 text-sm text-muted-foreground">
						{error || "The forwarding snapshot could not be loaded."}
					</p>
				</div>
				<Button
					type="button"
					variant="outline"
					className="mt-5"
					disabled={refreshing}
					onClick={onRefresh}
				>
					<RefreshCw
						className={cn("size-4", refreshing && "animate-spin")}
						aria-hidden="true"
					/>
					Refresh
				</Button>
			</div>
		</div>
	);
}

function ForwardingNavigation({
	configuredProfiles,
	historicalProfiles,
	selectedProfile,
	generatedAt,
	sampleLimit,
	navigationId,
	navigationOpen,
	error,
	refreshing,
	onRefresh,
	onSelectProfile,
	onClose,
}: {
	configuredProfiles: ProfileStatus[];
	historicalProfiles: ProfileStatus[];
	selectedProfile?: string;
	generatedAt: string;
	sampleLimit: number;
	navigationId: string;
	navigationOpen: boolean;
	error: string;
	onSelectProfile: (profile?: string) => void;
	onClose: () => void;
} & RefreshAction) {
	return (
		<div id={navigationId} className="flex h-full min-h-0 flex-col">
			<header className="flex shrink-0 items-center justify-between gap-3 border-b border-sidebar-border px-4 py-4">
				<div className="min-w-0">
					<p className="text-[11px] font-semibold tracking-[0.16em] text-sidebar-foreground/55 uppercase">
						Operations
					</p>
					<h2 className="mt-0.5 text-base font-semibold">Forwarding</h2>
				</div>
				<div className="flex items-center gap-1 md:hidden">
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className="hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
						aria-label="Refresh forwarding status"
						disabled={refreshing}
						onClick={onRefresh}
					>
						<RefreshCw
							className={cn(refreshing && "animate-spin")}
							aria-hidden="true"
						/>
					</Button>
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className="hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
						aria-label="Close forwarding navigation"
						aria-controls={navigationId}
						aria-expanded={navigationOpen}
						onClick={onClose}
					>
						<X aria-hidden="true" />
					</Button>
				</div>
			</header>

			{error ? (
				<RefreshErrorBanner error={error} className="md:hidden" />
			) : null}

			<nav
				className="min-h-0 flex-1 overflow-y-auto px-2 py-3"
				aria-label="Forwarding views"
			>
				<button
					type="button"
					aria-current={selectedProfile === undefined ? "page" : undefined}
					className={navigationRowClass(selectedProfile === undefined)}
					onClick={() => onSelectProfile(undefined)}
				>
					<span className="grid size-8 shrink-0 place-items-center rounded-lg border border-sidebar-border bg-sidebar text-sidebar-foreground/70">
						<LayoutDashboard className="size-4" aria-hidden="true" />
					</span>
					<span className="min-w-0">
						<span className="block text-sm font-medium">Overview</span>
						<span className="mt-0.5 block text-xs text-sidebar-foreground/55">
							All profile snapshots
						</span>
					</span>
				</button>

				<ProfileNavigationGroup
					title="Configured"
					profiles={configuredProfiles}
					emptyMessage="No configured profiles"
					selectedProfile={selectedProfile}
					onSelectProfile={onSelectProfile}
				/>
				<ProfileNavigationGroup
					title="Historical"
					profiles={historicalProfiles}
					emptyMessage="No retained historical profiles"
					selectedProfile={selectedProfile}
					onSelectProfile={onSelectProfile}
				/>
			</nav>

			<footer className="shrink-0 border-t border-sidebar-border px-4 py-3 text-[11px] leading-relaxed text-sidebar-foreground/55">
				<p>Snapshot generated</p>
				<time
					dateTime={generatedAt}
					className="block text-sidebar-foreground/75"
				>
					{formatTimestamp(generatedAt)}
				</time>
				<p className="mt-1">
					Up to {sampleLimit} retained {pluralize(sampleLimit, "attempt")} per
					profile
				</p>
			</footer>
		</div>
	);
}

function ProfileNavigationGroup({
	title,
	profiles,
	emptyMessage,
	selectedProfile,
	onSelectProfile,
}: {
	title: string;
	profiles: ProfileStatus[];
	emptyMessage: string;
	selectedProfile?: string;
	onSelectProfile: (profile: string) => void;
}) {
	return (
		<section className="mt-5" aria-labelledby={`forwarding-${title}-profiles`}>
			<div className="flex items-center justify-between px-2">
				<h3
					id={`forwarding-${title}-profiles`}
					className="text-[11px] font-semibold tracking-[0.14em] text-sidebar-foreground/50 uppercase"
				>
					{title}
				</h3>
				<span className="text-[11px] tabular-nums text-sidebar-foreground/45">
					{profiles.length}
				</span>
			</div>
			{profiles.length === 0 ? (
				<p className="px-2 py-3 text-xs text-sidebar-foreground/45">
					{emptyMessage}
				</p>
			) : (
				<ul className="mt-1 space-y-1">
					{profiles.map((profile) => (
						<li key={profile.profile_key}>
							<ProfileNavigationRow
								profile={profile}
								selected={selectedProfile === profile.profile_key}
								onSelect={() => onSelectProfile(profile.profile_key)}
							/>
						</li>
					))}
				</ul>
			)}
		</section>
	);
}

function ProfileNavigationRow({
	profile,
	selected,
	onSelect,
}: {
	profile: ProfileStatus;
	selected: boolean;
	onSelect: () => void;
}) {
	const latest = profile.samples[0];
	return (
		<button
			type="button"
			aria-current={selected ? "page" : undefined}
			className={navigationRowClass(selected)}
			onClick={onSelect}
		>
			<span className="min-w-0 flex-1">
				<span className="block break-all font-mono text-xs font-medium leading-5">
					{profile.profile_key}
				</span>
				<span className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]">
					<InlineProfileState profile={profile} />
					<span className="text-sidebar-foreground/25" aria-hidden="true">
						•
					</span>
					<InlineOutcomeStatus sample={latest} />
				</span>
			</span>
		</button>
	);
}

function navigationRowClass(selected: boolean) {
	return cn(
		"flex w-full items-start gap-3 rounded-xl border px-3 py-2.5 text-left transition-colors outline-none focus-visible:border-sidebar-ring focus-visible:ring-2 focus-visible:ring-sidebar-ring/40",
		selected
			? "border-sidebar-border bg-sidebar-accent text-sidebar-accent-foreground shadow-sm"
			: "border-transparent text-sidebar-foreground hover:bg-sidebar-accent/70 hover:text-sidebar-accent-foreground",
	);
}

function DetailHeader({
	selectedProfile,
	activeProfile,
	generatedAt,
	navigationId,
	navigationOpen,
	refreshing,
	onRefresh,
	onOpenNavigation,
}: {
	selectedProfile?: string;
	activeProfile?: ProfileStatus;
	generatedAt: string;
	navigationId: string;
	navigationOpen: boolean;
	onOpenNavigation: () => void;
} & RefreshAction) {
	const overview = selectedProfile === undefined;
	const subtitle = overview
		? "Configured profiles and retained attempt history"
		: activeProfile
			? "Retained forwarding attempts"
			: "Not present in the latest snapshot";

	return (
		<header className="flex shrink-0 items-center justify-between gap-3 border-b bg-background/95 px-3 py-3 backdrop-blur md:px-5 md:py-4">
			<div className="flex min-w-0 items-center gap-2.5">
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="md:hidden"
					aria-label="Open forwarding navigation"
					aria-controls={navigationId}
					aria-expanded={navigationOpen}
					onClick={onOpenNavigation}
				>
					<ChevronLeft aria-hidden="true" />
				</Button>
				<div className="grid size-9 shrink-0 place-items-center rounded-xl border bg-muted/50 text-muted-foreground">
					{overview ? (
						<LayoutDashboard className="size-4" aria-hidden="true" />
					) : activeProfile ? (
						<ServerCog className="size-4" aria-hidden="true" />
					) : (
						<CircleAlert className="size-4" aria-hidden="true" />
					)}
				</div>
				<div className="min-w-0">
					<h2
						className={cn(
							"text-sm font-semibold md:text-base",
							!overview && "break-all font-mono",
						)}
					>
						{overview ? "Overview" : selectedProfile}
					</h2>
					<p className="truncate text-xs text-muted-foreground">{subtitle}</p>
					<p className="mt-0.5 hidden text-[11px] text-muted-foreground sm:block">
						Last updated{" "}
						<time dateTime={generatedAt}>{formatTimestamp(generatedAt)}</time>
					</p>
				</div>
			</div>
			<Button
				type="button"
				variant="outline"
				size="sm"
				aria-label="Refresh forwarding status"
				disabled={refreshing}
				onClick={onRefresh}
			>
				<RefreshCw
					className={cn(refreshing && "animate-spin")}
					aria-hidden="true"
				/>
				<span className="hidden sm:inline">Refresh</span>
			</Button>
		</header>
	);
}

function RefreshErrorBanner({
	error,
	className,
}: {
	error: string;
	className?: string;
}) {
	return (
		<div
			className={cn(
				"shrink-0 border-b border-amber-300/60 bg-amber-50 px-4 py-3 text-amber-950 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-200",
				className,
			)}
		>
			<div className="flex gap-2" role="alert">
				<TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
				<div className="min-w-0 text-xs">
					<p className="font-medium">Refresh failed</p>
					<p className="mt-0.5 break-words opacity-80">
						Showing the previous snapshot. {error}
					</p>
				</div>
			</div>
		</div>
	);
}

function ForwardingOverview({
	profiles,
	sampleLimit,
	onSelectProfile,
}: {
	profiles: ProfileStatus[];
	sampleLimit: number;
	onSelectProfile: (profile: string) => void;
}) {
	let configuredCount = 0;
	let enabledCount = 0;
	let profilesWithAttempts = 0;
	for (const profile of profiles) {
		if (profile.configured) {
			configuredCount += 1;
			if (profile.enabled) enabledCount += 1;
		}
		if (profile.samples.length > 0) profilesWithAttempts += 1;
	}

	return (
		<div className="space-y-7 px-4 py-5 md:px-6 md:py-7 lg:px-8">
			<section aria-labelledby="forwarding-overview-heading">
				<div className="max-w-2xl">
					<p className="text-[11px] font-semibold tracking-[0.16em] text-muted-foreground uppercase">
						Current snapshot
					</p>
					<h3
						id="forwarding-overview-heading"
						className="mt-1 text-xl font-semibold tracking-tight"
					>
						Forwarding coverage
					</h3>
					<p className="mt-1 text-sm text-muted-foreground">
						Configuration state and retained attempt availability from the
						latest backend snapshot.
					</p>
				</div>

				<dl className="mt-5 grid overflow-hidden rounded-xl border bg-border sm:grid-cols-3">
					<OverviewMetric
						icon={SlidersHorizontal}
						label="Configured profiles"
						value={configuredCount}
					/>
					<OverviewMetric
						icon={Power}
						label="Enabled profiles"
						value={enabledCount}
					/>
					<OverviewMetric
						icon={History}
						label="Profiles with retained attempts"
						value={profilesWithAttempts}
					/>
				</dl>
			</section>

			<section aria-labelledby="profile-snapshot-heading">
				<div className="flex flex-wrap items-end justify-between gap-2">
					<div>
						<h3
							id="profile-snapshot-heading"
							className="text-base font-semibold"
						>
							Profile snapshot
						</h3>
						<p className="mt-0.5 text-xs text-muted-foreground">
							Latest retained outcome for each configured or historical profile.
						</p>
					</div>
					<p className="text-xs text-muted-foreground">
						Up to {sampleLimit} {pluralize(sampleLimit, "attempt")} per profile
					</p>
				</div>

				{profiles.length === 0 ? (
					<div className="mt-4 rounded-xl border border-dashed px-4 py-10 text-center">
						<CircleDashed
							className="mx-auto size-5 text-muted-foreground"
							aria-hidden="true"
						/>
						<p className="mt-3 text-sm font-medium">
							No forwarding profiles configured.
						</p>
						<p className="mt-1 text-xs text-muted-foreground">
							No retained historical profile attempts are available either.
						</p>
					</div>
				) : (
					<>
						<div className="mt-4 hidden overflow-hidden rounded-xl border md:block">
							<Table aria-label="Forwarding profile snapshot">
								<TableHeader className="bg-muted/40">
									<TableRow className="hover:bg-transparent">
										<TableHead className="pl-4">Profile</TableHead>
										<TableHead>State</TableHead>
										<TableHead>Latest outcome</TableHead>
										<TableHead>Latest completed</TableHead>
										<TableHead className="pr-4 text-right">Retained</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{profiles.map((profile) => (
										<ProfileSnapshotRow
											key={profile.profile_key}
											profile={profile}
											onSelect={() => onSelectProfile(profile.profile_key)}
										/>
									))}
								</TableBody>
							</Table>
						</div>

						<ul
							className="mt-4 divide-y overflow-hidden rounded-xl border md:hidden"
							aria-label="Forwarding profile snapshot"
						>
							{profiles.map((profile) => (
								<li key={profile.profile_key}>
									<MobileProfileSnapshot
										profile={profile}
										onSelect={() => onSelectProfile(profile.profile_key)}
									/>
								</li>
							))}
						</ul>
					</>
				)}
			</section>
		</div>
	);
}

function OverviewMetric({
	icon: Icon,
	label,
	value,
}: {
	icon: LucideIcon;
	label: string;
	value: number;
}) {
	return (
		<div className="flex items-center gap-3 bg-card px-4 py-4 sm:min-h-28 sm:items-start sm:justify-between sm:gap-4">
			<div className="min-w-0">
				<dt className="text-xs leading-5 text-muted-foreground">{label}</dt>
				<dd className="mt-0.5 text-2xl font-semibold tracking-tight tabular-nums">
					{value}
				</dd>
			</div>
			<span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
				<Icon className="size-4" aria-hidden="true" />
			</span>
		</div>
	);
}

function ProfileSnapshotRow({
	profile,
	onSelect,
}: {
	profile: ProfileStatus;
	onSelect: () => void;
}) {
	const latest = profile.samples[0];
	return (
		<TableRow>
			<TableCell className="max-w-72 whitespace-normal pl-4">
				<button
					type="button"
					className="break-all text-left font-mono text-xs font-medium underline-offset-4 outline-none hover:underline focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-ring/50"
					onClick={onSelect}
				>
					{profile.profile_key}
				</button>
			</TableCell>
			<TableCell>
				<ProfileStateBadge profile={profile} />
			</TableCell>
			<TableCell>
				{latest ? (
					<OutcomeBadge outcome={latest.outcome} />
				) : (
					<NoAttemptsStatus />
				)}
			</TableCell>
			<TableCell className="text-xs text-muted-foreground">
				{latest ? (
					<time dateTime={latest.completed_at}>
						{formatTimestamp(latest.completed_at)}
					</time>
				) : (
					"—"
				)}
			</TableCell>
			<TableCell className="pr-4 text-right font-mono text-xs tabular-nums">
				{profile.samples.length}
			</TableCell>
		</TableRow>
	);
}

function MobileProfileSnapshot({
	profile,
	onSelect,
}: {
	profile: ProfileStatus;
	onSelect: () => void;
}) {
	const latest = profile.samples[0];
	return (
		<button
			type="button"
			className="w-full px-4 py-4 text-left outline-none transition-colors hover:bg-muted/40 focus-visible:bg-muted/50 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50"
			onClick={onSelect}
		>
			<span className="flex items-start justify-between gap-3">
				<span className="min-w-0 break-all font-mono text-xs font-medium leading-5">
					{profile.profile_key}
				</span>
				<ProfileStateBadge profile={profile} />
			</span>
			<span className="mt-3 flex flex-wrap items-center justify-between gap-2">
				{latest ? (
					<span className="flex min-w-0 flex-wrap items-center gap-2">
						<OutcomeBadge outcome={latest.outcome} />
						<time
							dateTime={latest.completed_at}
							className="text-xs text-muted-foreground"
						>
							{formatTimestamp(latest.completed_at)}
						</time>
					</span>
				) : (
					<NoAttemptsStatus />
				)}
				<span className="text-[11px] text-muted-foreground">
					{profile.samples.length} retained
				</span>
			</span>
		</button>
	);
}

function ProfileDetail({
	profile,
	sampleLimit,
}: {
	profile: ProfileStatus;
	sampleLimit: number;
}) {
	const latest = profile.samples[0];
	const attemptsLabel = `Latest ${sampleLimit} ${pluralize(sampleLimit, "attempt")}`;

	return (
		<div className="space-y-7 px-4 py-5 md:px-6 md:py-7 lg:px-8">
			<section aria-labelledby="profile-state-heading">
				<p className="text-[11px] font-semibold tracking-[0.16em] text-muted-foreground uppercase">
					Profile snapshot
				</p>
				<h3 id="profile-state-heading" className="sr-only">
					State for {profile.profile_key}
				</h3>
				<dl className="mt-2 grid overflow-hidden rounded-xl border bg-border sm:grid-cols-3">
					<SummaryField label="State">
						<ProfileStateBadges profile={profile} />
					</SummaryField>
					<SummaryField label="Latest outcome">
						{latest ? (
							<OutcomeBadge outcome={latest.outcome} />
						) : (
							<NoAttemptsStatus />
						)}
					</SummaryField>
					<SummaryField label="Latest completed">
						{latest ? (
							<time
								dateTime={latest.completed_at}
								className="text-sm font-medium"
							>
								{formatTimestamp(latest.completed_at)}
							</time>
						) : (
							<span className="text-sm text-muted-foreground">—</span>
						)}
					</SummaryField>
				</dl>
			</section>

			<section aria-labelledby="attempt-history-heading">
				<div className="flex flex-wrap items-end justify-between gap-2">
					<div>
						<h3
							id="attempt-history-heading"
							className="text-base font-semibold"
						>
							{attemptsLabel}
						</h3>
						<p className="mt-0.5 text-xs text-muted-foreground">
							{profile.samples.length} retained in this snapshot
						</p>
					</div>
					<div className="flex items-center gap-1.5 text-xs text-muted-foreground">
						<Clock3 className="size-3.5" aria-hidden="true" />
						Newest first
					</div>
				</div>

				{profile.samples.length === 0 ? (
					<div className="mt-4 rounded-xl border border-dashed px-4 py-10 text-center">
						<CircleDashed
							className="mx-auto size-5 text-muted-foreground"
							aria-hidden="true"
						/>
						<p className="mt-3 text-sm font-medium">
							No forwarding attempts yet.
						</p>
						<p className="mt-1 text-xs text-muted-foreground">
							This snapshot contains no retained attempts for this profile.
						</p>
					</div>
				) : (
					<>
						<div className="mt-4 hidden overflow-hidden rounded-xl border md:block">
							<Table aria-label={`${attemptsLabel} for ${profile.profile_key}`}>
								<TableHeader className="bg-muted/40">
									<TableRow className="hover:bg-transparent">
										<TableHead className="pl-4">Attempt</TableHead>
										<TableHead>Completed</TableHead>
										<TableHead>Outcome</TableHead>
										<TableHead>Timing</TableHead>
										<TableHead className="pr-4">Error</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{profile.samples.map((sample, index) => (
										<AttemptTableRow
											key={attemptKey(sample, index)}
											sample={sample}
										/>
									))}
								</TableBody>
							</Table>
						</div>

						<ol
							className="mt-4 space-y-3 md:hidden"
							aria-label={`${attemptsLabel} for ${profile.profile_key}`}
						>
							{profile.samples.map((sample, index) => (
								<li key={attemptKey(sample, index)}>
									<MobileAttemptRecord sample={sample} />
								</li>
							))}
						</ol>
					</>
				)}
			</section>
		</div>
	);
}

function SummaryField({
	label,
	children,
}: {
	label: string;
	children: ReactNode;
}) {
	return (
		<div className="min-h-24 bg-card px-4 py-4">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className="mt-2 flex min-h-5 flex-wrap items-center gap-1.5">
				{children}
			</dd>
		</div>
	);
}

function AttemptTableRow({ sample }: { sample: ForwardAttemptSample }) {
	return (
		<TableRow>
			<TableCell className="pl-4 font-mono text-xs">
				<span className="tabular-nums">{sample.attempt_number}</span>
				{sample.is_retry ? (
					<Badge variant="outline" className="ml-2">
						Retry
					</Badge>
				) : null}
			</TableCell>
			<TableCell className="text-xs text-muted-foreground">
				<time dateTime={sample.completed_at}>
					{formatTimestamp(sample.completed_at)}
				</time>
			</TableCell>
			<TableCell>
				<OutcomeBadge outcome={sample.outcome} />
			</TableCell>
			<TableCell className="font-mono text-xs">
				{formatAttemptTiming(sample.dispatch_delay_ms, sample.latency_ms)}
			</TableCell>
			<TableCell className="max-w-64 whitespace-normal pr-4 font-mono text-xs text-muted-foreground">
				<span className="break-all">{sample.error_code ?? "—"}</span>
			</TableCell>
		</TableRow>
	);
}

function MobileAttemptRecord({ sample }: { sample: ForwardAttemptSample }) {
	return (
		<article className="rounded-xl border bg-card p-4">
			<header className="flex items-start justify-between gap-3">
				<div className="flex min-w-0 flex-wrap items-center gap-2">
					<p className="text-sm font-medium">
						Attempt{" "}
						<span className="font-mono tabular-nums">
							{sample.attempt_number}
						</span>
					</p>
					{sample.is_retry ? <Badge variant="outline">Retry</Badge> : null}
				</div>
				<OutcomeBadge outcome={sample.outcome} />
			</header>
			<dl className="mt-4 grid gap-3 text-xs">
				<div className="grid grid-cols-[5rem_minmax(0,1fr)] gap-3">
					<dt className="text-muted-foreground">Completed</dt>
					<dd className="text-right">
						<time dateTime={sample.completed_at}>
							{formatTimestamp(sample.completed_at)}
						</time>
					</dd>
				</div>
				<div className="grid grid-cols-[5rem_minmax(0,1fr)] gap-3">
					<dt className="text-muted-foreground">Timing</dt>
					<dd className="text-right font-mono">
						{formatAttemptTiming(sample.dispatch_delay_ms, sample.latency_ms)}
					</dd>
				</div>
				<div className="grid grid-cols-[5rem_minmax(0,1fr)] gap-3">
					<dt className="text-muted-foreground">Error</dt>
					<dd className="break-all text-right font-mono text-muted-foreground">
						{sample.error_code ?? "—"}
					</dd>
				</div>
			</dl>
		</article>
	);
}

function UnavailableProfile({
	profileKey,
	onOverview,
}: {
	profileKey: string;
	onOverview: () => void;
}) {
	return (
		<div className="grid min-h-full place-items-center px-5 py-12">
			<div className="w-full max-w-lg rounded-xl border border-dashed bg-card px-5 py-10 text-center">
				<div className="mx-auto grid size-11 place-items-center rounded-full bg-muted text-muted-foreground">
					<CircleAlert className="size-5" aria-hidden="true" />
				</div>
				<h3 className="mt-4 text-base font-semibold">Profile unavailable</h3>
				<p className="mt-2 text-sm text-muted-foreground">
					The profile{" "}
					<code className="break-all rounded bg-muted px-1 py-0.5 text-xs text-foreground">
						{profileKey}
					</code>{" "}
					is not present in the latest forwarding snapshot.
				</p>
				<Button
					type="button"
					variant="outline"
					className="mt-5"
					onClick={onOverview}
				>
					<LayoutDashboard aria-hidden="true" />
					View Overview
				</Button>
			</div>
		</div>
	);
}

function ProfileStateBadges({ profile }: { profile: ProfileStatus }) {
	if (!profile.configured) {
		return <ProfileStateBadge profile={profile} />;
	}
	return (
		<>
			<Badge variant="outline">
				<ServerCog aria-hidden="true" />
				Configured
			</Badge>
			<ProfileStateBadge profile={profile} />
		</>
	);
}

function ProfileStateBadge({ profile }: { profile: ProfileStatus }) {
	const { label, Icon, badgeClassName } = profileStatePresentation(profile);
	return (
		<Badge variant="outline" className={badgeClassName}>
			<Icon aria-hidden="true" />
			{label}
		</Badge>
	);
}

function InlineProfileState({ profile }: { profile: ProfileStatus }) {
	const { label, Icon, inlineClassName } = profileStatePresentation(profile);
	return (
		<span className={cn("inline-flex items-center gap-1", inlineClassName)}>
			<Icon className="size-3" aria-hidden="true" />
			{label}
		</span>
	);
}

function profileStatePresentation(profile: ProfileStatus): {
	label: string;
	Icon: LucideIcon;
	badgeClassName: string;
	inlineClassName: string;
} {
	if (!profile.configured) {
		return {
			label: "Historical",
			Icon: Archive,
			badgeClassName:
				"border-slate-300 bg-slate-50 text-slate-700 dark:border-slate-700 dark:bg-slate-900/70 dark:text-slate-300",
			inlineClassName: "text-sidebar-foreground/65",
		};
	}
	if (profile.enabled) {
		return {
			label: "Enabled",
			Icon: Power,
			badgeClassName:
				"border-emerald-300/70 bg-emerald-50 text-emerald-800 dark:border-emerald-800 dark:bg-emerald-950/50 dark:text-emerald-300",
			inlineClassName: "text-emerald-700 dark:text-emerald-300",
		};
	}
	return {
		label: "Disabled",
		Icon: CircleOff,
		badgeClassName: "bg-muted/50 text-muted-foreground",
		inlineClassName: "text-sidebar-foreground/55",
	};
}

function OutcomeBadge({ outcome }: { outcome: ForwardAttemptOutcome }) {
	const { label, Icon, badgeClassName } = outcomePresentation(outcome);
	return (
		<Badge variant="outline" className={badgeClassName}>
			<Icon aria-hidden="true" />
			{label}
		</Badge>
	);
}

function InlineOutcomeStatus({
	sample,
}: {
	sample: ForwardAttemptSample | undefined;
}) {
	if (!sample) {
		return (
			<span className="inline-flex items-center gap-1 text-sidebar-foreground/50">
				<CircleDashed className="size-3" aria-hidden="true" />
				No attempts
			</span>
		);
	}
	const { label, Icon, inlineClassName } = outcomePresentation(sample.outcome);
	return (
		<span className={cn("inline-flex items-center gap-1", inlineClassName)}>
			<Icon className="size-3" aria-hidden="true" />
			Latest: {label}
		</span>
	);
}

function NoAttemptsStatus() {
	return (
		<span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
			<CircleDashed className="size-3.5" aria-hidden="true" />
			No attempts
		</span>
	);
}

function outcomePresentation(outcome: ForwardAttemptOutcome): {
	label: string;
	Icon: LucideIcon;
	badgeClassName: string;
	inlineClassName: string;
} {
	switch (outcome) {
		case "success":
			return {
				label: "Success",
				Icon: CheckCircle2,
				badgeClassName:
					"border-emerald-300/70 bg-emerald-50 text-emerald-800 dark:border-emerald-800 dark:bg-emerald-950/50 dark:text-emerald-300",
				inlineClassName: "text-emerald-700 dark:text-emerald-300",
			};
		case "transient_failure":
			return {
				label: "Transient failure",
				Icon: TriangleAlert,
				badgeClassName:
					"border-amber-300/70 bg-amber-50 text-amber-900 dark:border-amber-800 dark:bg-amber-950/50 dark:text-amber-300",
				inlineClassName: "text-amber-700 dark:text-amber-300",
			};
		case "permanent_failure":
			return {
				label: "Permanent failure",
				Icon: CircleX,
				badgeClassName:
					"border-red-300/70 bg-red-50 text-red-800 dark:border-red-800 dark:bg-red-950/50 dark:text-red-300",
				inlineClassName: "text-red-700 dark:text-red-300",
			};
	}
}

function attemptKey(sample: ForwardAttemptSample, index: number) {
	return JSON.stringify([
		sample.attempt_number,
		sample.started_at,
		sample.completed_at,
		index,
	]);
}

function formatTimestamp(value: string) {
	const date = new Date(value);
	return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function formatLatency(ms: number): string {
	if (ms < 1000) return `${ms}ms`;
	return `${(ms / 1000).toFixed(1)}s`;
}

function formatAttemptTiming(
	dispatchDelayMs: number | null | undefined,
	requestLatencyMs: number,
): string {
	const dispatch =
		dispatchDelayMs == null ? "—" : formatLatency(dispatchDelayMs);
	return `Dispatch ${dispatch} · Request ${formatLatency(requestLatencyMs)}`;
}

function pluralize(count: number, singular: string) {
	return count === 1 ? singular : `${singular}s`;
}

function errorMessage(error: unknown) {
	return error instanceof Error
		? error.message
		: "The forwarding snapshot could not be loaded.";
}
