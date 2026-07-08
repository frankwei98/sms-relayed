import { createFileRoute } from "@tanstack/react-router";
import { ConfigEditor } from "#/components/config/config-editor";

export const Route = createFileRoute("/config")({
	component: ConfigPage,
});

function ConfigPage() {
	return <ConfigEditor />;
}
