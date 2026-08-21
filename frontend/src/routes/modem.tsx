import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/modem")({
	beforeLoad: () => {
		throw redirect({
			to: "/status",
			search: { section: "modem" },
		});
	},
});
