// @vitest-environment jsdom

import {
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
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
	cleanup();
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

describe("MessageConsole timeline pagination", () => {
	test("loads the latest ten messages, then prepends an older cursor page", async () => {
		const messageRequests: string[] = [];
		const latestPage = Array.from({ length: 10 }, (_, index) => ({
			...unreadMessage,
			id: 100 - index,
			body: index === 0 ? "latest message" : `recent-${index}`,
			timestamp: `2026-07-${String(20 - index).padStart(2, "0")}T00:00:00Z`,
			read_at: "2026-07-20T00:01:00Z",
		}));
		const olderMessage = {
			...unreadMessage,
			id: 90,
			body: "older message",
			timestamp: "2026-07-10T00:00:00Z",
			read_at: "2026-07-20T00:01:00Z",
		};

		mocks.apiFetch.mockImplementation((input: string) => {
			if (input === "/api/conversations") {
				return Promise.resolve([
					{
						phone_number: unreadMessage.phone_number,
						last_message: latestPage[0],
						unread_count: 0,
						total_count: 11,
					},
				]);
			}
			if (input.startsWith("/api/messages?")) {
				messageRequests.push(input);
				const params = new URL(input, "http://localhost").searchParams;
				if (params.has("before_timestamp")) {
					return Promise.resolve([olderMessage]);
				}
				return Promise.resolve(
					params.get("limit") === "11"
						? [...latestPage, olderMessage]
						: latestPage,
				);
			}
			return Promise.resolve({});
		});

		render(<MessageConsole />);
		fireEvent.click(
			await screen.findByRole("button", { name: /\+15550000001/ }),
		);
		await waitFor(() => expect(messageRequests.length).toBeGreaterThan(0));

		const firstPageUrl = messageRequests[0];
		expect(firstPageUrl).toContain("limit=10");

		fireEvent.click(
			await screen.findByRole("button", { name: "Load older messages" }),
		);
		await screen.findByText("older message");

		const cursorRequest = messageRequests.find((input) =>
			input.includes("before_timestamp"),
		);
		const cursorParams = new URL(cursorRequest ?? "", "http://localhost")
			.searchParams;
		expect(cursorParams.get("before_timestamp")).toBe("2026-07-11T00:00:00Z");
		expect(cursorParams.get("before_id")).toBe("91");

		const requestCountBeforeSend = messageRequests.length;
		fireEvent.change(screen.getByPlaceholderText("Message"), {
			target: { value: "reply" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Send message" }));
		await waitFor(() =>
			expect(messageRequests.length).toBeGreaterThan(requestCountBeforeSend),
		);
		const refreshParams = new URL(
			messageRequests.at(-1) ?? "",
			"http://localhost",
		).searchParams;
		expect(refreshParams.get("limit")).toBe("11");
	});
});
