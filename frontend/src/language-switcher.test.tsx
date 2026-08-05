// @vitest-environment jsdom

import {
	act,
	cleanup,
	fireEvent,
	render,
	screen,
	within,
} from "@testing-library/react";
import type { ComponentType, ReactNode } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { Route } from "#/routes/__root";

const mocks = vi.hoisted(() => ({
	apiFetch: vi.fn(),
	navigate: vi.fn(),
}));

vi.mock("#/lib/api", () => ({
	apiFetch: mocks.apiFetch,
	AUTH_UNAUTHORIZED_EVENT: "sms-relayed:unauthorized",
}));
vi.mock("@tanstack/react-devtools", () => ({
	TanStackDevtools: () => null,
}));
vi.mock("@tanstack/react-router-devtools", () => ({
	TanStackRouterDevtoolsPanel: () => null,
}));
vi.mock("@tanstack/react-router", () => ({
	createRootRoute: (options: unknown) => ({ options }),
	Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
	Outlet: () => null,
	useLocation: () => ({ pathname: "/" }),
	useNavigate: () => mocks.navigate,
}));

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe("LanguageSwitcher", () => {
	test("opens the complete language menu without a Base UI context error", async () => {
		let resolveAuth!: (value: { authenticated: boolean }) => void;
		mocks.apiFetch.mockReturnValue(
			new Promise<{ authenticated: boolean }>((resolve) => {
				resolveAuth = resolve;
			}),
		);
		const RootComponent = Route.options.component as ComponentType;

		render(<RootComponent />);
		await act(async () => {
			resolveAuth({ authenticated: true });
		});
		const trigger = screen.getByRole("button", {
			name: "Language",
		});
		fireEvent.keyDown(trigger, { key: "ArrowDown" });

		const menu = await screen.findByRole("menu");
		expect(within(menu).getByText("English")).toBeTruthy();
		expect(within(menu).getByText("简体中文")).toBeTruthy();
		expect(within(menu).getByText("日本語")).toBeTruthy();
		expect(within(menu).getByText("한국어")).toBeTruthy();
		expect(within(menu).getByText("Français")).toBeTruthy();
		expect(within(menu).getByText("Español")).toBeTruthy();
	});
});
