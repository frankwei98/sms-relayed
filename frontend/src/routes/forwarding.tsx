import { createFileRoute } from "@tanstack/react-router";
import { ForwardingStatusPanel } from "#/components/forwarding/forwarding-status-panel";

export const Route = createFileRoute("/forwarding")({
	component: ForwardingPage,
});

function ForwardingPage() {
	return <ForwardingStatusPanel />;
}
