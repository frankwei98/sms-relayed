import { createFileRoute } from "@tanstack/react-router";
import { ForwardingStatusPanel } from "#/components/forwarding/forwarding-status-panel";

type ForwardingSearch = {
	profile?: string;
};

export const Route = createFileRoute("/forwarding")({
	validateSearch: (search: Record<string, unknown>): ForwardingSearch => ({
		profile: typeof search.profile === "string" ? search.profile : undefined,
	}),
	component: ForwardingPage,
});

function ForwardingPage() {
	const { profile } = Route.useSearch();
	const navigate = Route.useNavigate();

	return (
		<ForwardingStatusPanel
			selectedProfile={profile}
			onSelectProfile={(nextProfile) =>
				navigate({ search: { profile: nextProfile } })
			}
		/>
	);
}
