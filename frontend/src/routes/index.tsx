import { createFileRoute } from "@tanstack/react-router";
import { MessageConsole } from "#/components/messages/message-console";

export const Route = createFileRoute("/")({ component: Home });

function Home() {
	return <MessageConsole />;
}
