import { createFileRoute } from "@tanstack/react-router";
import { Cpu, Forward, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ForwardingStatusPanel } from "#/components/forwarding/forwarding-status-panel";
import { ModemStatusPanel } from "#/components/modem/modem-status-panel";
import { cn } from "#/lib/utils";

type StatusSection = "modem" | "forwarding";

type StatusSearch = {
	section?: StatusSection;
	profile?: string;
};

const statusSections = [
	{ key: "modem", label: "status.tabs.modem", icon: Cpu },
	{ key: "forwarding", label: "status.tabs.forwarding", icon: Forward },
] as const satisfies ReadonlyArray<{
	key: StatusSection;
	label: `status.tabs.${string}`;
	icon: LucideIcon;
}>;

export const Route = createFileRoute("/status")({
	validateSearch: (search: Record<string, unknown>): StatusSearch => ({
		section:
			search.section === "forwarding" || search.section === "modem"
				? search.section
				: undefined,
		profile: typeof search.profile === "string" ? search.profile : undefined,
	}),
	component: StatusPage,
});

function StatusPage() {
	const { section, profile } = Route.useSearch();
	const navigate = Route.useNavigate();
	const { t } = useTranslation();
	const activeSection = section ?? "modem";

	function changeSection(nextSection: StatusSection) {
		void navigate({
			search: {
				section: nextSection,
				profile,
			},
		});
	}

	return (
		<div className="flex h-full min-h-0 flex-col gap-3 md:gap-4">
			<header className="flex shrink-0 flex-col gap-3 px-1 md:flex-row md:items-end md:justify-between md:px-2">
				<div>
					<p className="text-[0.68rem] font-semibold uppercase tracking-[0.2em] text-muted-foreground">
						{t("status.eyebrow")}
					</p>
					<h2 className="text-2xl font-semibold tracking-tight">
						{t("status.title")}
					</h2>
					<p className="mt-1 max-w-xl text-sm text-muted-foreground">
						{t("status.description")}
					</p>
				</div>

				<div
					className="inline-flex w-fit items-center gap-1 rounded-xl border bg-muted/60 p-1"
					role="tablist"
					aria-label={t("status.ariaTabs")}
				>
					{statusSections.map(({ key, label, icon: Icon }) => {
						const isActive = activeSection === key;

						return (
							<button
								key={key}
								type="button"
								id={`status-tab-${key}`}
								role="tab"
								aria-selected={isActive}
								aria-controls="status-panel"
								onClick={() => changeSection(key)}
								className={cn(
									"inline-flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
									isActive && "bg-background text-foreground shadow-sm",
								)}
							>
								<Icon className="size-4" aria-hidden="true" />
								{t(label)}
							</button>
						);
					})}
				</div>
			</header>

			<div
				id="status-panel"
				role="tabpanel"
				aria-labelledby={`status-tab-${activeSection}`}
				className="min-h-0 flex-1"
			>
				{activeSection === "modem" ? (
					<div className="h-full overflow-y-auto px-1 pb-6 md:px-2 md:pb-2">
						<ModemStatusPanel />
					</div>
				) : (
					<ForwardingStatusPanel
						selectedProfile={profile}
						onSelectProfile={(nextProfile) =>
							navigate({
								search: { section: "forwarding", profile: nextProfile },
							})
						}
					/>
				)}
			</div>
		</div>
	);
}
