import { createFileRoute } from "@tanstack/react-router";
import { MessageConsole } from "#/components/messages/message-console";

type MessageSearch = {
	phone?: string;
	message?: number;
};

export const Route = createFileRoute("/")({
	validateSearch: (search): MessageSearch => ({
		phone: typeof search.phone === "string" ? search.phone : undefined,
		message:
			typeof search.message === "number"
				? search.message
				: typeof search.message === "string" && /^\d+$/.test(search.message)
					? Number(search.message)
					: undefined,
	}),
	component: Home,
});

function Home() {
	const search = Route.useSearch();
	return (
		<MessageConsole
			initialPhone={search.phone}
			targetMessageId={search.message}
		/>
	);
}
