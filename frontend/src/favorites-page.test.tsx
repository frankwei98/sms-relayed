// @vitest-environment jsdom

import {
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { FavoritesPage } from "#/components/messages/favorites-page";
import i18n from "#/lib/i18n";

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

const favorite = {
	id: 12,
	direction: "inbound" as const,
	phone_number: "+15550000001",
	body: "Saved verification code",
	timestamp: "2026-07-11T00:00:00Z",
	status: "received" as const,
	source: "modem" as const,
	modem_sms_path: null,
	read_at: "2026-07-11T00:00:00Z",
	error: null,
	created_at: "2026-07-11T00:00:00Z",
	updated_at: "2026-08-01T00:00:00Z",
	favorite_at: "2026-08-01T00:00:00Z",
	delete_blocked: false,
};

afterEach(async () => {
	cleanup();
	vi.restoreAllMocks();
	vi.clearAllMocks();
	mocks.handlers = {};
	await i18n.changeLanguage("en");
});

describe("FavoritesPage", () => {
	test("opens the original message and removes a favorite from its context menu", async () => {
		const onOpenMessage = vi.fn();
		mocks.apiFetch.mockImplementation((input: string, init?: RequestInit) => {
			if (input === "/api/messages/favorites")
				return Promise.resolve([favorite]);
			if (input === "/api/messages/12/unfavorite" && init?.method === "POST") {
				return Promise.resolve({ ...favorite, favorite_at: null });
			}
			return Promise.resolve({});
		});

		render(<FavoritesPage onOpenMessage={onOpenMessage} />);

		const card = await screen.findByRole("button", {
			name: /Saved verification code/,
		});
		expect(screen.getByText("+15550000001")).toBeTruthy();
		fireEvent.click(card);
		expect(onOpenMessage).toHaveBeenCalledWith("+15550000001", 12);

		fireEvent.contextMenu(card);
		fireEvent.click(
			await screen.findByRole("menuitem", { name: "Remove from favorites" }),
		);

		await waitFor(() => {
			expect(mocks.apiFetch).toHaveBeenCalledWith(
				"/api/messages/12/unfavorite",
				{ method: "POST" },
			);
		});
		expect(screen.queryByText("Saved verification code")).toBeNull();
	});

	test("confirms before deleting a favorite message", async () => {
		mocks.apiFetch.mockImplementation((input: string, init?: RequestInit) => {
			if (input === "/api/messages/favorites")
				return Promise.resolve([favorite]);
			if (input === "/api/messages/12" && init?.method === "DELETE") {
				return Promise.resolve({ deleted: 12 });
			}
			return Promise.resolve({});
		});

		render(<FavoritesPage onOpenMessage={vi.fn()} />);
		const card = await screen.findByRole("button", {
			name: /Saved verification code/,
		});
		fireEvent.contextMenu(card);
		fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }));
		expect(
			screen.getByRole("dialog", { name: "Delete message?" }),
		).toBeTruthy();
		fireEvent.click(screen.getByRole("button", { name: "Delete message" }));

		await waitFor(() => {
			expect(mocks.apiFetch).toHaveBeenCalledWith("/api/messages/12", {
				method: "DELETE",
			});
		});
	});
});
