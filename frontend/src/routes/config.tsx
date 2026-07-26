import { createFileRoute } from "@tanstack/react-router";
import { ConfigEditor } from "#/components/config/config-editor";
import {
	type ConfigSection,
	isConfigSection,
} from "#/components/config/config-sections";

type ConfigSearch = {
	section: ConfigSection;
};

export const Route = createFileRoute("/config")({
	validateSearch: (search: Record<string, unknown>): ConfigSearch => ({
		section: isConfigSection(search.section) ? search.section : "device",
	}),
	component: ConfigPage,
});

function ConfigPage() {
	const { section } = Route.useSearch();
	const navigate = Route.useNavigate();

	return (
		<ConfigEditor
			section={section}
			onSectionChange={(nextSection) =>
				navigate({ search: { section: nextSection } })
			}
		/>
	);
}
