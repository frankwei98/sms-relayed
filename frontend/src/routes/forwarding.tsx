import { createFileRoute, redirect } from "@tanstack/react-router";

type ForwardingSearch = {
	profile?: string;
};

export const Route = createFileRoute("/forwarding")({
	validateSearch: (search: Record<string, unknown>): ForwardingSearch => ({
		profile: typeof search.profile === "string" ? search.profile : undefined,
	}),
	beforeLoad: ({ search }) => {
		throw redirect({
			to: "/status",
			search: { section: "forwarding", profile: search.profile },
		});
	},
});
