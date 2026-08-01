import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { FavoritesPage } from "#/components/messages/favorites-page";

export const Route = createFileRoute("/favorites")({
	component: FavoritesRoute,
});

function FavoritesRoute() {
	const navigate = useNavigate();
	return (
		<FavoritesPage
			onOpenMessage={(phone, messageId) =>
				navigate({ to: "/", search: { phone, message: messageId } })
			}
		/>
	);
}
