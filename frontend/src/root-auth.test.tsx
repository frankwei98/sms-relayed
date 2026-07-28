// @vitest-environment jsdom

import {
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { Route } from "#/routes/__root";

const mocks = vi.hoisted(() => ({
	apiFetch: vi.fn(),
	navigate: vi.fn(),
	pathname: "/",
}));

vi.mock("#/lib/api", () => ({
	apiFetch: mocks.apiFetch,
	AUTH_UNAUTHORIZED_EVENT: "sms-relayed:unauthorized",
}));

vi.mock("@tanstack/react-router", async () => {
	const actual = await vi.importActual<typeof import("@tanstack/react-router")>(
		"@tanstack/react-router",
	);
	return {
		...actual,
		createRootRoute: (options: unknown) => ({ options }),
		Link: ({ children }: { children: React.ReactNode }) => (
			<a href="/">{children}</a>
		),
		Outlet: () => <div>Sensitive dashboard</div>,
		useLocation: () => ({ pathname: mocks.pathname }),
		useNavigate: () => mocks.navigate,
	};
});

vi.mock("@tanstack/react-devtools", () => ({
	TanStackDevtools: () => null,
}));
vi.mock("@tanstack/react-router-devtools", () => ({
	TanStackRouterDevtoolsPanel: () => null,
}));

afterEach(() => {
	cleanup();
	mocks.apiFetch.mockReset();
	mocks.navigate.mockReset();
	mocks.pathname = "/";
});

describe("root authentication lifecycle", () => {
	test("an unauthorized API response removes the protected dashboard", async () => {
		mocks.apiFetch.mockResolvedValue({ authenticated: true });
		const RootComponent = Route.options.component;

		render(<RootComponent />);
		expect(await screen.findByText("Sensitive dashboard")).toBeTruthy();

		window.dispatchEvent(new Event("sms-relayed:unauthorized"));

		await waitFor(() =>
			expect(screen.queryByText("Sensitive dashboard")).toBeNull(),
		);
		expect(mocks.navigate).toHaveBeenCalledWith({ to: "/login" });
	});

	test("logout ends the server session and removes the protected dashboard", async () => {
		mocks.apiFetch.mockImplementation((input: string) =>
			Promise.resolve(
				input === "/api/auth/me" ? { authenticated: true } : undefined,
			),
		);
		const RootComponent = Route.options.component;

		render(<RootComponent />);
		fireEvent.click(await screen.findByRole("button", { name: "Logout" }));

		await waitFor(() =>
			expect(mocks.apiFetch).toHaveBeenCalledWith("/api/auth/logout", {
				method: "POST",
			}),
		);
		expect(screen.queryByText("Sensitive dashboard")).toBeNull();
		expect(mocks.navigate).toHaveBeenCalledWith({ to: "/login" });
	});

	test("logout removes the protected dashboard when the server request fails", async () => {
		mocks.apiFetch.mockImplementation((input: string) =>
			input === "/api/auth/me"
				? Promise.resolve({ authenticated: true })
				: Promise.reject(new Error("backend unavailable")),
		);
		const RootComponent = Route.options.component;

		render(<RootComponent />);
		fireEvent.click(await screen.findByRole("button", { name: "Logout" }));

		await waitFor(() =>
			expect(screen.queryByText("Sensitive dashboard")).toBeNull(),
		);
		expect(mocks.navigate).toHaveBeenCalledWith({ to: "/login" });
	});
});
