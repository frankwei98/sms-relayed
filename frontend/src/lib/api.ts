export type ApiErrorBody = { error: { code: string; message: string } };

export async function apiFetch<T>(
	input: RequestInfo | URL,
	init?: RequestInit,
): Promise<T> {
	const response = await fetch(input, {
		credentials: "include",
		headers: {
			"Content-Type": "application/json",
			...(init?.headers ?? {}),
		},
		...init,
	});
	if (!response.ok) {
		const body = (await response
			.json()
			.catch(() => null)) as ApiErrorBody | null;
		throw new Error(
			body?.error.message ?? `Request failed: ${response.status}`,
		);
	}
	return (await response.json()) as T;
}

export type AuthState = { authenticated: boolean };
export type MessageDirection = "inbound" | "outbound";
export type MessageStatus = "received" | "sending" | "sent" | "failed";
export type MessageSource = "modem" | "web" | "cli";

export type Message = {
	id: number;
	direction: MessageDirection;
	phone_number: string;
	body: string;
	timestamp: string;
	status: MessageStatus;
	source: MessageSource;
	modem_sms_path: string | null;
	read_at: string | null;
	error: string | null;
	created_at: string;
	updated_at: string;
};

export type ConversationSummary = {
	phone_number: string;
	last_message: Message;
	unread_count: number;
	total_count: number;
};
