// @vitest-environment jsdom

import {
	act,
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
	handlers: {} as Record<string, (payload?: unknown) => void>,
}));

vi.mock("#/lib/api", () => ({ apiFetch: mocks.apiFetch }));
vi.mock("#/lib/events", () => ({
	subscribeEvents: (handlers: Record<string, (payload?: unknown) => void>) => {
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
	vi.useRealTimers();
	vi.restoreAllMocks();
	vi.clearAllMocks();
	mocks.handlers = {};
});

describe("MessageConsole refresh fallback", () => {
	test("periodically reloads conversations when SSE events are missed", async () => {
		vi.useFakeTimers();
		let conversationLoads = 0;
		mocks.apiFetch.mockImplementation((input: string) => {
			if (input === "/api/conversations") {
				conversationLoads += 1;
				return Promise.resolve([]);
			}
			return Promise.resolve([]);
		});

		render(<MessageConsole />);
		await act(async () => {
			await Promise.resolve();
		});
		expect(conversationLoads).toBe(1);

		await act(async () => {
			await vi.advanceTimersByTimeAsync(30_100);
		});
		expect(conversationLoads).toBe(2);
	});
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
					Number(params.get("limit")) > 10
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
		expect(refreshParams.get("limit")).toBe("12");

		fireEvent.click(screen.getByRole("button", { name: /\+15550000001/ }));
		expect(screen.getByText("older message")).toBeTruthy();
	});

	test("keeps server ordering when browser and SQLite timestamp parsing differ", async () => {
		const messageRequests: string[] = [];
		const validMessages = Array.from({ length: 9 }, (_, index) => ({
			...unreadMessage,
			id: 200 - index,
			body: `valid-${index}`,
			timestamp: `2026-07-${String(20 - index).padStart(2, "0")}T00:00:00Z`,
			created_at: `2026-07-${String(20 - index).padStart(2, "0")}T00:00:01Z`,
			read_at: "2026-07-20T00:01:00Z",
		}));
		const malformedMessage = {
			...unreadMessage,
			id: 191,
			body: "malformed timestamp",
			timestamp: "2026-07-30T00:00:00+0800",
			created_at: "2026-07-10T00:00:00Z",
			read_at: "2026-07-20T00:01:00Z",
		};

		mocks.apiFetch.mockImplementation((input: string) => {
			if (input === "/api/conversations") {
				return Promise.resolve([
					{
						phone_number: unreadMessage.phone_number,
						last_message: validMessages[0],
						unread_count: 0,
						total_count: 11,
					},
				]);
			}
			if (input.startsWith("/api/messages?")) {
				messageRequests.push(input);
				const params = new URL(input, "http://localhost").searchParams;
				return Promise.resolve(
					params.has("before_timestamp")
						? []
						: [...validMessages, malformedMessage],
				);
			}
			return Promise.resolve({});
		});

		render(<MessageConsole />);
		fireEvent.click(
			await screen.findByRole("button", { name: /\+15550000001/ }),
		);
		fireEvent.click(
			await screen.findByRole("button", { name: "Load older messages" }),
		);

		await waitFor(() => expect(messageRequests.length).toBe(2));
		expect(screen.queryAllByText("2026-07-30T00:00:00+0800")).toHaveLength(0);
		const cursorParams = new URL(messageRequests[1], "http://localhost")
			.searchParams;
		expect(cursorParams.get("before_timestamp")).toBe(
			"2026-07-30T00:00:00+0800",
		);
		expect(cursorParams.get("before_id")).toBe("191");
	});

	test("scrolls to the latest message when reopening the same conversation", async () => {
		vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(
			1000,
		);
		mocks.apiFetch.mockImplementation((input: string) => {
			if (input === "/api/conversations") {
				return Promise.resolve([
					{
						phone_number: unreadMessage.phone_number,
						last_message: {
							...unreadMessage,
							read_at: unreadMessage.timestamp,
						},
						unread_count: 0,
						total_count: 1,
					},
				]);
			}
			if (input.startsWith("/api/messages?")) {
				return Promise.resolve([
					{ ...unreadMessage, read_at: unreadMessage.timestamp },
				]);
			}
			return Promise.resolve({});
		});

		render(<MessageConsole />);
		fireEvent.click(
			await screen.findByRole("button", { name: /\+15550000001/ }),
		);
		const firstTimeline = await screen.findByRole("log", {
			name: "Message timeline",
		});
		let firstScrollTop = 0;
		Object.defineProperty(firstTimeline, "scrollTop", {
			configurable: true,
			get: () => firstScrollTop,
			set: (value) => {
				firstScrollTop = value;
			},
		});
		await waitFor(() => expect(firstTimeline.scrollTop).toBe(1000));

		fireEvent.click(
			screen.getByRole("button", { name: "Back to conversations" }),
		);
		fireEvent.click(screen.getByRole("button", { name: /\+15550000001/ }));
		const reopenedTimeline = await screen.findByRole("log", {
			name: "Message timeline",
		});
		let reopenedScrollTop = 0;
		Object.defineProperty(reopenedTimeline, "scrollTop", {
			configurable: true,
			get: () => reopenedScrollTop,
			set: (value) => {
				reopenedScrollTop = value;
			},
		});
		await waitFor(() => expect(reopenedTimeline.scrollTop).toBe(1000));
	});

	test("refreshes a window larger than the server page cap in cursor chunks", async () => {
		const allMessages = Array.from({ length: 510 }, (_, index) => ({
			...unreadMessage,
			id: 1000 - index,
			body: index === 509 ? "oldest retained message" : `message-${index}`,
			timestamp: new Date(
				Date.UTC(2026, 0, 1, 0, 0, 510 - index),
			).toISOString(),
			read_at: "2026-07-20T00:01:00Z",
		}));
		const messageRequests: string[] = [];
		let initialWindowLoaded = false;

		mocks.apiFetch.mockImplementation((input: string) => {
			if (input === "/api/conversations") {
				return Promise.resolve([
					{
						phone_number: unreadMessage.phone_number,
						last_message: allMessages[0],
						unread_count: 0,
						total_count: allMessages.length,
					},
				]);
			}
			if (input.startsWith("/api/messages?")) {
				messageRequests.push(input);
				const params = new URL(input, "http://localhost").searchParams;
				if (!initialWindowLoaded) {
					initialWindowLoaded = true;
					return Promise.resolve(allMessages);
				}
				if (params.has("before_timestamp")) {
					return Promise.resolve(allMessages.slice(500));
				}
				return Promise.resolve(allMessages.slice(0, 500));
			}
			return Promise.resolve({});
		});

		render(<MessageConsole />);
		fireEvent.click(
			await screen.findByRole("button", { name: /\+15550000001/ }),
		);
		await screen.findByText("oldest retained message");

		const requestsBeforeRefresh = messageRequests.length;
		mocks.handlers["message.updated"]?.();
		await waitFor(
			() =>
				expect(messageRequests.length).toBeGreaterThan(requestsBeforeRefresh),
			{ timeout: 2000 },
		);
		await waitFor(() => {
			const refreshLimits = messageRequests
				.slice(requestsBeforeRefresh)
				.map((input) =>
					new URL(input, "http://localhost").searchParams.get("limit"),
				);
			expect(refreshLimits).toEqual(["500", "10"]);
		});
		expect(screen.getByText("oldest retained message")).toBeTruthy();
	});

	test("grows the refreshed window when a new message arrives", async () => {
		const initialMessages = Array.from({ length: 10 }, (_, index) => ({
			...unreadMessage,
			id: 300 - index,
			body: index === 9 ? "oldest visible" : `initial-${index}`,
			timestamp: `2026-07-${String(20 - index).padStart(2, "0")}T00:00:00Z`,
			read_at: unreadMessage.timestamp,
		}));
		const newMessage = {
			...unreadMessage,
			id: 301,
			body: "newly arrived",
			timestamp: "2026-07-21T00:00:00Z",
			read_at: unreadMessage.timestamp,
		};
		const messageRequests: string[] = [];
		let serverMessages = initialMessages;

		mocks.apiFetch.mockImplementation((input: string) => {
			if (input === "/api/conversations") {
				return Promise.resolve([
					{
						phone_number: unreadMessage.phone_number,
						last_message: serverMessages[0],
						unread_count: 0,
						total_count: serverMessages.length,
					},
				]);
			}
			if (input.startsWith("/api/messages?")) {
				messageRequests.push(input);
				const limit = Number(
					new URL(input, "http://localhost").searchParams.get("limit"),
				);
				return Promise.resolve(serverMessages.slice(0, limit));
			}
			return Promise.resolve({});
		});

		render(<MessageConsole />);
		fireEvent.click(
			await screen.findByRole("button", { name: /\+15550000001/ }),
		);
		await screen.findByText("oldest visible");

		const requestsBeforeOtherPhone = messageRequests.length;
		mocks.handlers["message.created"]?.({
			payload: { ...newMessage, phone_number: "+15550000002" },
		});
		await waitFor(() =>
			expect(messageRequests.length).toBeGreaterThan(requestsBeforeOtherPhone),
		);
		const otherPhoneRefresh = new URL(
			messageRequests.at(-1) ?? "",
			"http://localhost",
		).searchParams;
		expect(otherPhoneRefresh.get("limit")).toBe("10");

		const historicalMessage = {
			...unreadMessage,
			id: 280,
			body: "historical import",
			timestamp: "2020-01-01T00:00:00Z",
			read_at: unreadMessage.timestamp,
		};
		serverMessages = [...initialMessages, historicalMessage];
		const requestsBeforeImport = messageRequests.length;
		mocks.handlers["message.created"]?.({ payload: historicalMessage });
		await waitFor(() =>
			expect(messageRequests.length).toBeGreaterThan(requestsBeforeImport),
		);
		expect(screen.queryByText("historical import")).toBeNull();
		expect(screen.getByText("oldest visible")).toBeTruthy();

		serverMessages = [newMessage, ...initialMessages, historicalMessage];
		const requestsBeforeRefresh = messageRequests.length;
		mocks.handlers["message.created"]?.({ payload: newMessage });
		await screen.findAllByText("newly arrived");

		const refreshParams = new URL(
			messageRequests.at(requestsBeforeRefresh) ?? "",
			"http://localhost",
		).searchParams;
		expect(refreshParams.get("limit")).toBe("11");
		expect(screen.getByText("oldest visible")).toBeTruthy();
	});

	test("keeps both refreshed and older messages when requests overlap", async () => {
		const latestPage = Array.from({ length: 10 }, (_, index) => ({
			...unreadMessage,
			id: 400 - index,
			body: `overlap-${index}`,
			timestamp: `2026-07-${String(20 - index).padStart(2, "0")}T00:00:00Z`,
			read_at: unreadMessage.timestamp,
		}));
		const newMessage = {
			...unreadMessage,
			id: 401,
			body: "arrived during paging",
			timestamp: "2026-07-21T00:00:00Z",
			read_at: unreadMessage.timestamp,
		};
		const olderMessage = {
			...unreadMessage,
			id: 390,
			body: "older during refresh",
			timestamp: "2026-07-10T00:00:00Z",
			read_at: unreadMessage.timestamp,
		};
		let initialLoaded = false;
		let refreshCalls = 0;
		let resolveRefresh: ((messages: typeof latestPage) => void) | undefined;

		mocks.apiFetch.mockImplementation((input: string) => {
			if (input === "/api/conversations") {
				return Promise.resolve([
					{
						phone_number: unreadMessage.phone_number,
						last_message: newMessage,
						unread_count: 0,
						total_count: 12,
					},
				]);
			}
			if (input.startsWith("/api/messages?")) {
				const params = new URL(input, "http://localhost").searchParams;
				if (params.has("before_timestamp")) {
					return Promise.resolve([olderMessage]);
				}
				if (!initialLoaded) {
					initialLoaded = true;
					return Promise.resolve(latestPage);
				}
				if (refreshCalls++ > 0) {
					return Promise.resolve([newMessage, ...latestPage, olderMessage]);
				}
				return new Promise((resolve) => {
					resolveRefresh = resolve;
				});
			}
			return Promise.resolve({});
		});

		render(<MessageConsole />);
		fireEvent.click(
			await screen.findByRole("button", { name: /\+15550000001/ }),
		);
		await screen.findByText("overlap-0");

		mocks.handlers["message.created"]?.({ payload: newMessage });
		await waitFor(() => expect(resolveRefresh).toBeTypeOf("function"));
		fireEvent.click(
			screen.getByRole("button", { name: "Load older messages" }),
		);
		await screen.findByText("older during refresh");
		resolveRefresh?.([newMessage, ...latestPage]);

		await screen.findByText("arrived during paging");
		expect(screen.getByText("older during refresh")).toBeTruthy();
	});
});
