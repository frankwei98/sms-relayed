// @vitest-environment jsdom

import { beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiFetch: vi.fn(),
	listeners: new Map<string, EventListener>(),
}));

vi.mock("./api", () => ({
	AUTH_UNAUTHORIZED_EVENT: "sms-relayed:unauthorized",
	apiFetch: mocks.apiFetch,
}));

class FakeEventSource {
	addEventListener(name: string, listener: EventListener) {
		mocks.listeners.set(name, listener);
	}

	close() {}
}

beforeEach(() => {
	mocks.apiFetch.mockReset();
	mocks.listeners.clear();
	vi.stubGlobal("EventSource", FakeEventSource);
});

test("an SSE error clears authentication when the session expired", async () => {
	const unauthorized = vi.fn();
	window.addEventListener("sms-relayed:unauthorized", unauthorized);
	mocks.apiFetch.mockResolvedValue({ authenticated: false });
	const { subscribeEvents } = await import("./events");
	subscribeEvents({});

	mocks.listeners.get("error")?.(new Event("error"));
	await vi.waitFor(() => expect(unauthorized).toHaveBeenCalledOnce());

	window.removeEventListener("sms-relayed:unauthorized", unauthorized);
});
