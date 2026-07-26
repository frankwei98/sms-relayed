import { useBlocker, useNavigate } from "@tanstack/react-router";
import {
	AlertTriangle,
	CheckCircle2,
	ChevronLeft,
	CircleDot,
	ListTree,
	LoaderCircle,
	RotateCcw,
	Save,
	ShieldAlert,
	XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ConfigSectionEditor } from "#/components/config/config-section-editors";
import {
	CONFIG_SECTION_DEFINITIONS,
	type ConfigSection,
} from "#/components/config/config-sections";
import { useConfigDraft } from "#/components/config/use-config-draft";
import { Badge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "#/components/ui/dialog";
import { WorkspaceLayout } from "#/components/ui/workspace-layout";
import { useAuth } from "#/lib/auth";
import {
	type ConfigDocument,
	type ConfigWarning,
	loadConfigDocument,
	scheduleRestart,
} from "#/lib/config-api";

type ConfigEditorProps = {
	section: ConfigSection;
	onSectionChange: (section: ConfigSection) => void;
};

export function ConfigEditor({ section, onSectionChange }: ConfigEditorProps) {
	const [document, setDocument] = useState<ConfigDocument | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState("");

	const load = useCallback(async () => {
		setLoading(true);
		setError("");
		try {
			setDocument(await loadConfigDocument());
		} catch (loadError) {
			setError((loadError as Error).message);
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		void load();
	}, [load]);

	if (loading && !document) {
		return (
			<div className="flex h-full items-center justify-center text-sm text-muted-foreground">
				<LoaderCircle className="mr-2 size-4 animate-spin" />
				Loading configuration…
			</div>
		);
	}

	if (!document) {
		return (
			<div className="flex h-full items-center justify-center p-6">
				<div className="max-w-md rounded-xl border border-destructive/30 bg-destructive/5 p-5">
					<h2 className="font-semibold">Configuration unavailable</h2>
					<p className="mt-2 text-sm text-muted-foreground">{error}</p>
					<Button
						className="mt-4"
						variant="outline"
						onClick={() => void load()}
					>
						Retry
					</Button>
				</div>
			</div>
		);
	}

	return (
		<ConfigWorkspace
			key={document.revision}
			initialDocument={document}
			section={section}
			onSectionChange={onSectionChange}
			onReload={load}
			backgroundError={error}
		/>
	);
}

type ConfigWorkspaceProps = ConfigEditorProps & {
	initialDocument: ConfigDocument;
	onReload: () => Promise<void>;
	backgroundError: string;
};

function ConfigWorkspace({
	initialDocument,
	section,
	onSectionChange,
	onReload,
	backgroundError,
}: ConfigWorkspaceProps) {
	const draft = useConfigDraft(initialDocument);
	const { setAuth } = useAuth();
	const navigate = useNavigate();
	const [mobileNavigationOpen, setMobileNavigationOpen] = useState(false);
	const [restartOpen, setRestartOpen] = useState(false);
	const [restartBusy, setRestartBusy] = useState(false);
	const [actionMessage, setActionMessage] = useState("");
	const allowNavigation = useRef(false);
	const activeSection = useMemo(
		() =>
			CONFIG_SECTION_DEFINITIONS.find(
				(definition) => definition.id === section,
			) ?? CONFIG_SECTION_DEFINITIONS[0],
		[section],
	);

	const blocker = useBlocker({
		shouldBlockFn: ({ current, next }) =>
			!allowNavigation.current &&
			draft.isDirty &&
			current.pathname === "/config" &&
			next.pathname !== "/config",
		enableBeforeUnload: draft.isDirty,
		disabled: !draft.isDirty,
		withResolver: true,
	});

	async function handleConfirmSave() {
		const result = await draft.confirmSave();
		if (!result) return;
		if (result.session_invalidated) {
			allowNavigation.current = true;
			try {
				setAuth({ authenticated: false });
				await navigate({
					to: "/login",
					search: {
						notice: "config_saved_restart_scheduled",
					},
				});
			} finally {
				allowNavigation.current = false;
			}
			return;
		}
		setActionMessage(
			result.requires_restart
				? "Configuration saved. Restart required."
				: "Configuration saved.",
		);
	}

	async function handleRestart() {
		setRestartBusy(true);
		setActionMessage("");
		try {
			await scheduleRestart();
			setActionMessage(
				"Restart scheduled. The dashboard may disconnect briefly.",
			);
			setRestartOpen(false);
		} catch (restartError) {
			setActionMessage(`Restart failed: ${(restartError as Error).message}`);
		} finally {
			setRestartBusy(false);
		}
	}

	const navigation = (
		<>
			<div className="flex shrink-0 items-center justify-between border-b px-4 py-3">
				<div>
					<p className="text-sm font-semibold">Configuration</p>
					<p className="text-xs text-muted-foreground">
						{draft.dirtySections.size > 0
							? `${draft.dirtySections.size} ${
									draft.dirtySections.size === 1 ? "category" : "categories"
								} changed`
							: "No unsaved changes"}
					</p>
				</div>
				<Button
					variant="ghost"
					size="icon-sm"
					className="md:hidden"
					onClick={() => setMobileNavigationOpen(false)}
					aria-label="Back to configuration"
				>
					<ChevronLeft className="size-4" />
				</Button>
			</div>
			<nav
				className="min-h-0 flex-1 overflow-y-auto p-2"
				aria-label="Config categories"
			>
				{CONFIG_SECTION_DEFINITIONS.map((definition) => {
					const Icon = definition.icon;
					const active = definition.id === section;
					const dirtySection = draft.dirtySections.has(definition.id);
					return (
						<button
							type="button"
							key={definition.id}
							aria-current={active ? "page" : undefined}
							className={`mb-1 flex w-full items-start gap-3 rounded-lg px-3 py-2.5 text-left transition-colors ${
								active
									? "bg-sidebar-accent text-sidebar-accent-foreground"
									: "text-sidebar-foreground/75 hover:bg-sidebar-accent/70 hover:text-sidebar-accent-foreground"
							}`}
							onClick={() => {
								onSectionChange(definition.id);
								setMobileNavigationOpen(false);
							}}
						>
							<Icon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
							<span className="min-w-0 flex-1">
								<span className="flex items-center gap-2 text-sm font-medium">
									{definition.label}
									{dirtySection ? (
										<CircleDot
											className="size-3 text-amber-600 dark:text-amber-400"
											aria-label="Unsaved changes"
										/>
									) : null}
								</span>
								<span className="mt-0.5 block text-xs leading-snug text-muted-foreground">
									{definition.description}
								</span>
							</span>
						</button>
					);
				})}
			</nav>
		</>
	);

	return (
		<WorkspaceLayout
			navigation={navigation}
			navigationLabel="Configuration categories"
			mobileNavigationOpen={mobileNavigationOpen}
		>
			<div className="flex shrink-0 flex-col border-b bg-background/95 supports-backdrop-filter:backdrop-blur">
				<div className="flex items-center gap-3 px-3 py-2 md:px-5 md:py-3">
					<Button
						variant="outline"
						size="sm"
						className="md:hidden"
						onClick={() => setMobileNavigationOpen(true)}
					>
						<ListTree className="size-4" />
						Categories
					</Button>
					<div className="min-w-0 flex-1">
						<p className="truncate text-sm font-semibold">
							{activeSection.label}
						</p>
						<p className="truncate text-xs text-muted-foreground">
							{draft.isDirty ? "Unsaved draft" : "Saved configuration"}
							{draft.restartRequired ? " · Restart required" : ""}
						</p>
					</div>
					<div className="flex shrink-0 items-center gap-2">
						<Button
							size="sm"
							onClick={() => void draft.openPreview()}
							disabled={!draft.isDirty || draft.preview.status === "loading"}
						>
							<Save className="size-4" />
							Save
						</Button>
						<Button
							variant="outline"
							size="sm"
							onClick={() => {
								setActionMessage("");
								void draft.runCheck();
							}}
							disabled={draft.check.status === "checking"}
						>
							{draft.check.status === "checking" ? (
								<LoaderCircle className="size-4 animate-spin" />
							) : (
								<CheckCircle2 className="size-4" />
							)}
							Check
						</Button>
						<Button
							variant="destructive"
							size="sm"
							onClick={() => setRestartOpen(true)}
						>
							<RotateCcw className="size-4" />
							Restart
						</Button>
					</div>
				</div>
				<ConfigStatusLine
					check={draft.check}
					message={backgroundError || actionMessage}
				/>
			</div>

			<div key={section} className="min-h-0 flex-1 overflow-y-auto">
				<ConfigSectionEditor
					section={section}
					config={draft.draft}
					onConfigChange={(config) => {
						setActionMessage("");
						draft.updateDraft(config);
					}}
					onPathChange={(path, value) => {
						setActionMessage("");
						draft.updatePath(path, value);
					}}
				/>
			</div>

			<SaveReviewDialog
				preview={draft.preview}
				onClose={draft.closePreview}
				onConfirm={() => void handleConfirmSave()}
				onReload={() => void onReload()}
			/>

			<Dialog open={restartOpen} onOpenChange={setRestartOpen}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Schedule service restart?</DialogTitle>
						<DialogDescription>
							The request only schedules the service-manager command. This page
							may disconnect before the service is available again.
						</DialogDescription>
					</DialogHeader>
					{draft.isDirty ? (
						<div className="flex gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-sm">
							<AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-700 dark:text-amber-400" />
							<p>
								Unsaved edits are only in this browser tab. Restart uses the
								persisted file and may make this draft unrecoverable.
							</p>
						</div>
					) : null}
					<DialogFooter>
						<Button variant="outline" onClick={() => setRestartOpen(false)}>
							Cancel
						</Button>
						<Button
							variant="destructive"
							disabled={restartBusy}
							onClick={() => void handleRestart()}
						>
							{restartBusy ? (
								<LoaderCircle className="size-4 animate-spin" />
							) : (
								<RotateCcw className="size-4" />
							)}
							Schedule restart
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			<Dialog
				open={blocker.status === "blocked"}
				onOpenChange={(open) => {
					if (!open && blocker.status === "blocked") blocker.reset();
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Leave with unsaved changes?</DialogTitle>
						<DialogDescription>
							The configuration draft contains credentials and is intentionally
							not stored in the browser. Leaving will discard it.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => blocker.status === "blocked" && blocker.reset()}
						>
							Stay
						</Button>
						<Button
							variant="destructive"
							onClick={() => blocker.status === "blocked" && blocker.proceed()}
						>
							Discard and leave
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</WorkspaceLayout>
	);
}

function ConfigStatusLine({
	check,
	message,
}: {
	check: ReturnType<typeof useConfigDraft>["check"];
	message: string;
}) {
	let checkText = "Not checked";
	let className = "text-muted-foreground";
	if (check.status === "checking") checkText = "Checking complete draft…";
	if (check.status === "passed") {
		checkText = "Check passed";
		className = "text-emerald-700 dark:text-emerald-400";
	}
	if (check.status === "failed") {
		checkText = `Check failed: ${check.message}`;
		className = "text-destructive";
	}
	const checkInProgressOrFailed =
		check.status === "checking" || check.status === "failed";
	return (
		<div
			className={`min-h-7 border-t px-3 py-1.5 text-xs md:px-5 ${className}`}
			aria-live="polite"
		>
			{checkInProgressOrFailed ? checkText : message || checkText}
		</div>
	);
}

function SaveReviewDialog({
	preview,
	onClose,
	onConfirm,
	onReload,
}: {
	preview: ReturnType<typeof useConfigDraft>["preview"];
	onClose: () => void;
	onConfirm: () => void;
	onReload: () => void;
}) {
	const open = preview.status !== "closed";
	const warningLabels: Record<ConfigWarning, string> = {
		password_change:
			"All sessions will be signed out after Save + Restart is scheduled.",
		api_disable: "The dashboard will be unavailable after restart.",
		api_endpoint_change: "The dashboard address may change after restart.",
		database_path_change:
			"The service will use a different message database after restart.",
	};

	return (
		<Dialog
			open={open}
			onOpenChange={(nextOpen) => {
				if (!nextOpen && preview.status !== "ready") onClose();
				if (!nextOpen && preview.status === "ready" && !preview.saving)
					onClose();
			}}
		>
			<DialogContent
				className="flex max-h-[calc(100dvh-1rem)] w-[min(72rem,calc(100%-1rem))] max-w-none grid-rows-none flex-col gap-0 overflow-hidden p-0 sm:max-w-none"
				showCloseButton={preview.status !== "ready" || !preview.saving}
			>
				<DialogHeader className="shrink-0 border-b px-4 py-4 pr-12 md:px-6">
					<DialogTitle>Review configuration changes</DialogTitle>
					<DialogDescription>
						Check the exact TOML that will replace the current file, then
						confirm a second time to save.
					</DialogDescription>
				</DialogHeader>

				<div className="min-h-0 flex-1 overflow-y-auto p-4 md:p-6">
					{preview.status === "loading" ? (
						<div className="flex min-h-64 items-center justify-center text-sm text-muted-foreground">
							<LoaderCircle className="mr-2 size-4 animate-spin" />
							Generating TOML diff and checking the draft…
						</div>
					) : null}

					{preview.status === "error" ? (
						<div className="mx-auto max-w-xl space-y-4 py-10 text-center">
							<XCircle className="mx-auto size-8 text-destructive" />
							<div>
								<h3 className="font-semibold">
									{preview.conflict
										? "Configuration changed on disk"
										: "Preview failed"}
								</h3>
								<p className="mt-1 text-sm text-muted-foreground">
									{preview.message}
								</p>
							</div>
							{preview.conflict ? (
								<Button variant="destructive" onClick={onReload}>
									Reload from disk and discard draft
								</Button>
							) : null}
						</div>
					) : null}

					{preview.status === "ready" ? (
						<div className="space-y-4">
							<div
								className={`flex items-start gap-3 rounded-lg border p-3 ${
									preview.response.check.passed
										? "border-emerald-500/30 bg-emerald-500/10"
										: "border-destructive/30 bg-destructive/10"
								}`}
							>
								{preview.response.check.passed ? (
									<CheckCircle2 className="mt-0.5 size-4 shrink-0 text-emerald-700 dark:text-emerald-400" />
								) : (
									<XCircle className="mt-0.5 size-4 shrink-0 text-destructive" />
								)}
								<div>
									<p className="text-sm font-medium">
										{preview.response.check.passed
											? "Check passed"
											: "Check failed"}
									</p>
									{preview.response.check.message ? (
										<p className="mt-1 text-xs text-muted-foreground">
											{preview.response.check.message}
										</p>
									) : null}
								</div>
							</div>

							<div className="flex items-start gap-3 rounded-lg border border-amber-500/30 bg-amber-500/10 p-3">
								<ShieldAlert className="mt-0.5 size-4 shrink-0 text-amber-700 dark:text-amber-400" />
								<p className="text-xs leading-relaxed">
									This diff is intentionally unredacted. Passwords, tokens,
									webhook URLs, and other credentials are visible in this
									authenticated dialog and the network response.
								</p>
							</div>

							{preview.response.warnings.length > 0 ? (
								<div className="space-y-2 rounded-lg border p-3">
									<p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
										Operational warnings
									</p>
									{preview.response.warnings.map((warning) => (
										<div key={warning} className="flex gap-2 text-sm">
											<AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-700 dark:text-amber-400" />
											<span>{warningLabels[warning] ?? warning}</span>
										</div>
									))}
								</div>
							) : null}

							<div>
								<div className="mb-2 flex items-center justify-between gap-3">
									<h3 className="text-sm font-semibold">TOML diff</h3>
									<Badge variant="outline">
										{preview.response.requires_restart
											? "Restart required"
											: "No runtime change"}
									</Badge>
								</div>
								{preview.response.has_changes ? (
									<textarea
										readOnly
										wrap="off"
										aria-label="Complete plaintext TOML diff"
										value={preview.response.diff}
										className="h-[50dvh] w-full resize-none overflow-auto rounded-lg border bg-zinc-950 p-4 font-mono text-xs leading-relaxed text-zinc-100 outline-none focus-visible:ring-2 focus-visible:ring-ring"
									/>
								) : (
									<p className="rounded-lg border p-4 text-sm text-muted-foreground">
										No file changes to save.
									</p>
								)}
							</div>
							{preview.saveError ? (
								<p className="text-sm text-destructive" role="alert">
									Save failed: {preview.saveError}
								</p>
							) : null}
						</div>
					) : null}
				</div>

				<DialogFooter className="shrink-0 border-t bg-popover px-4 py-3 md:px-6">
					<Button
						variant="outline"
						disabled={preview.status === "ready" && preview.saving}
						onClick={onClose}
					>
						Cancel
					</Button>
					{preview.status === "ready" ? (
						<Button
							disabled={
								preview.saving ||
								!preview.response.check.passed ||
								!preview.response.has_changes
							}
							onClick={onConfirm}
						>
							{preview.saving ? (
								<LoaderCircle className="size-4 animate-spin" />
							) : (
								<Save className="size-4" />
							)}
							{preview.response.password_change_pending
								? "Save and schedule restart"
								: "Save configuration"}
						</Button>
					) : null}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
