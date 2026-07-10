// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { MessageConsole } from "#/components/messages/message-console";

const mocks = vi.hoisted(() => ({
	apiFetch: vi.fn(),
	handlers: {} as Record<string, () => void>,
}));

vi.mock("#/lib/api", () => ({ apiFetch: mocks.apiFetch }));
vi.mock("#/lib/events", () => ({
	subscribeEvents: (handlers: Record<string, () => void>) => {
		mocks.handlers = handlers;
		return () => {};
	},
}));

const unreadMessage = {
	id: 1,
	direction: "inbound" as const,
	phone_number: "+15550000001",
	body: "test body",
	timestamp: "2026-07-11T00:00:00Z",
	status: "received" as const,
	source: "modem" as const,
	modem_sms_path: null,
	read_at: null,
	error: null,
	created_at: "2026-07-11T00:00:00Z",
	updated_at: "2026-07-11T00:00:00Z",
};

afterEach(() => {
	vi.clearAllMocks();
	mocks.handlers = {};
});

describe("MessageConsole bulk read", () => {
	test("keeps one mutation in flight while SSE refreshes arrive", async () => {
		let resolveRead: (() => void) | undefined;
		let markedRead = false;
		mocks.apiFetch.mockImplementation((input: string, init?: RequestInit) => {
			if (input === "/api/conversations") {
				return Promise.resolve([
					{
						phone_number: unreadMessage.phone_number,
						last_message: {
							...unreadMessage,
							read_at: markedRead ? "2026-07-11T00:01:00Z" : null,
						},
						unread_count: markedRead ? 0 : 1,
						total_count: 1,
					},
				]);
			}
			if (input.startsWith("/api/messages?")) {
				return Promise.resolve([
					{
						...unreadMessage,
						read_at: markedRead ? "2026-07-11T00:01:00Z" : null,
					},
				]);
			}
			if (input.includes("/api/conversations/") && init?.method === "POST") {
				return new Promise((resolve) => {
					resolveRead = () => {
						markedRead = true;
						resolve({ changed: 1 });
					};
				});
			}
			return Promise.resolve({});
		});

		render(<MessageConsole />);
		const conversation = await screen.findByRole("button", {
			name: /\+15550000001/,
		});
		fireEvent.click(conversation);

		await waitFor(() => {
			expect(
				mocks.apiFetch.mock.calls.filter(
					([input, init]) =>
						String(input).includes("/api/conversations/") &&
						(init as RequestInit | undefined)?.method === "POST",
				).length,
			).toBe(1);
		});

		mocks.handlers["message.updated"]?.();
		mocks.handlers["message.read_state_changed"]?.();
		await new Promise((resolve) => setTimeout(resolve, 150));
		expect(
			mocks.apiFetch.mock.calls.filter(
				([input, init]) =>
					String(input).includes("/api/conversations/") &&
					(init as RequestInit | undefined)?.method === "POST",
			).length,
		).toBe(1);

		resolveRead?.();
		await waitFor(() => expect(markedRead).toBe(true));
	});
});
