// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest";
import { LOGIN_NOTICES, Route } from "#/routes/login";

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
	routerMocks.search = {};
	routerMocks.navigate.mockReset();
});

describe("Login notice", () => {
	test("rejects arbitrary notice text from the URL", () => {
		expect(
			Route.options.validateSearch({ notice: "Trust this external message" }),
		).toEqual({});
	});

	test("rejects inherited object property names as notice codes", () => {
		expect(Route.options.validateSearch({ notice: "toString" })).toEqual({});
	});

	test("accepts the controlled post-save notice code with fixed copy", () => {
		const notice = "config_saved_restart_scheduled";
		expect(Route.options.validateSearch({ notice })).toEqual({
			notice,
		});
		expect(LOGIN_NOTICES[notice]).toBe(
			"Configuration saved and restart scheduled. Sign in with the new password after the service returns.",
		);
	});
});
