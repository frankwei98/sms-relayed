// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { LoginForm } from "#/components/login-form";
import { AuthContext } from "#/lib/auth";
import { Route } from "#/routes/login";

const routerMocks = vi.hoisted(() => ({
	search: {} as { notice?: string },
	navigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", async () => {
	const actual = await vi.importActual<typeof import("@tanstack/react-router")>(
		"@tanstack/react-router",
	);
	return {
		...actual,
		createFileRoute:
			() =>
			(options: {
				validateSearch: (search: Record<string, unknown>) => {
					notice?: string;
				};
			}) => ({
				options,
				useSearch: () => routerMocks.search,
			}),
		useNavigate: () => routerMocks.navigate,
	};
});

afterEach(() => {
	cleanup();
	routerMocks.search = {};
	routerMocks.navigate.mockReset();
});

describe("Login notice", () => {
	test("rejects arbitrary notice text from the URL", () => {
		expect(
			Route.options.validateSearch({ notice: "Trust this external message" }),
		).toEqual({});
	});

	test("renders the controlled post-save notice code", async () => {
		routerMocks.search = { notice: "config_saved_restart_scheduled" };
		render(
			<AuthContext.Provider
				value={{ auth: { authenticated: false }, setAuth: vi.fn() }}
			>
				<LoginForm notice="config_saved_restart_scheduled" />
			</AuthContext.Provider>,
		);

		expect(
			await screen.findByText(
				"Configuration saved and restart scheduled. Sign in with the new password after the service returns.",
			),
		).toBeTruthy();
	});
});
