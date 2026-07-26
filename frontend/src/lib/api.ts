import { captureFailure } from "./monitoring";

export type ApiErrorBody = { error: { code: string; message: string } };

export class ApiRequestError extends Error {
	readonly status: number;
	readonly code: string;

	constructor(status: number, code: string, message: string) {
		super(message);
		this.name = "ApiRequestError";
		this.status = status;
		this.code = code;
	}
}

export type ApiResponse<T> = {
	data: T;
	response: Response;
};

export async function apiRequest<T>(
	input: RequestInfo | URL,
	init?: RequestInit,
): Promise<ApiResponse<T>> {
	let response: Response;
	try {
		response = await fetch(input, {
			credentials: "include",
			...init,
			headers: {
				"Content-Type": "application/json",
				...(init?.headers ?? {}),
			},
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
		throw new ApiRequestError(
			response.status,
			body?.error.code ?? "request_failed",
			body?.error.message ?? `Request failed: ${response.status}`,
		);
	}
	const body = await response.text();
	return {
		data: body.trim().length === 0 ? (undefined as T) : (JSON.parse(body) as T),
		response,
	};
}

export async function apiFetch<T>(
	input: RequestInfo | URL,
	init?: RequestInit,
): Promise<T> {
	return (await apiRequest<T>(input, init)).data;
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
	configured: boolean;
	enabled: boolean;
	samples: ForwardAttemptSample[];
};

export type ForwardingResponse = {
	generated_at: string;
	sample_limit: number;
	profiles: ProfileStatus[];
};
