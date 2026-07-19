import { captureFailure } from "./monitoring";

export type ApiErrorBody = { error: { code: string; message: string } };

export async function apiFetch<T>(
	input: RequestInfo | URL,
	init?: RequestInit,
): Promise<T> {
	let response: Response;
	try {
		response = await fetch(input, {
			credentials: "include",
			headers: {
				"Content-Type": "application/json",
				...(init?.headers ?? {}),
			},
			...init,
		});
	} catch (error) {
		captureFailure("api.request_failed", { status: "network_error" });
		throw error;
	}
	if (!response.ok) {
		if (response.status >= 500) {
			captureFailure("api.request_failed", {
				status: response.status.toString(),
			});
		}
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

export type ForwardAttemptOutcome =
	| "success"
	| "transient_failure"
	| "permanent_failure";

export type ForwardAttemptSample = {
	attempt_number: number;
	is_retry: boolean;
	started_at: string;
	completed_at: string;
	latency_ms: number;
	dispatch_delay_ms: number | null;
	outcome: ForwardAttemptOutcome;
	error_code: string | null;
};

export type ProfileStatus = {
	profile_key: string;
	enabled: boolean;
	samples: ForwardAttemptSample[];
};

export type ForwardingResponse = {
	generated_at: string;
	profiles: ProfileStatus[];
};
