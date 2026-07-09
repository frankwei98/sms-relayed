import { createFileRoute } from "@tanstack/react-router";
import { ModemStatusPanel } from "#/components/modem/modem-status-panel";

export const Route = createFileRoute("/modem")({
	component: ModemPage,
});

function ModemPage() {
	return <ModemStatusPanel />;
}
