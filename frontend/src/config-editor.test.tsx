// @vitest-environment jsdom

import {
	act,
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ConfigEditor } from "#/components/config/config-editor";
import { ConfigSectionEditor } from "#/components/config/config-section-editors";
import type { ConfigSection } from "#/components/config/config-sections";
import { AuthContext } from "#/lib/auth";
import type { AppConfig } from "#/lib/config-model";

const routerMocks = vi.hoisted(() => ({
	navigate: vi.fn(),
	shouldBlockFn: vi.fn(),
	blockerStatus: "idle" as "idle" | "blocked",
	proceed: vi.fn(),
	reset: vi.fn(),
}));

vi.mock("@tanstack/react-router", async () => {
	const actual = await vi.importActual<typeof import("@tanstack/react-router")>(
		"@tanstack/react-router",
	);
	return {
		...actual,
		useNavigate: () => routerMocks.navigate,
		useBlocker: ({
			shouldBlockFn,
		}: {
			shouldBlockFn: (navigation: {
				current: { pathname: string };
				next: { pathname: string };
			}) => boolean;
		}) => {
			routerMocks.shouldBlockFn.mockImplementation(shouldBlockFn);
			return {
				status: routerMocks.blockerStatus,
				current: undefined,
				next: undefined,
				action: undefined,
				proceed: routerMocks.proceed,
				reset: routerMocks.reset,
			};
		},
	};
});

const baseConfig: AppConfig = {
	app: {
		device_name: "relay-one",
		modem_path: "/org/freedesktop/ModemManager1/Modem/0",
	},
	sms: {
		ignore_storage: ["sm"],
		code_keywords: ["code"],
	},
	forward: { enabled: ["telegram.alerts"] },
	delivery: { concurrency: 2 },
	channels: {
		bark: {},
		telegram: {
			alerts: {
				bot_token: "telegram-secret",
				chat_id: "123",
				api_base: "https://api.telegram.org",
			},
		},
		wecom: {},
		dingtalk: {},
		lark: {},
		shell: {},
	},
	api: {
		enabled: true,
		bind: "0.0.0.0",
		port: 8080,
		enable_ipv6: false,
		password: "old-password",
		database_path: "/tmp/sms-relayed.sqlite",
	},
	http: {
		connect_timeout_secs: 5,
		request_timeout_secs: 30,
		shell_timeout_secs: 30,
	},
	retention: {
		enabled: false,
		max_age_days: 90,
		batch_size: 500,
	},
};

type PreviewOverrides = Partial<{
	passed: boolean;
	message: string | null;
	diff: string;
	hasChanges: boolean;
	passwordChange: boolean;
	warnings: string[];
}>;

function previewResponse(previewOverrides: PreviewOverrides = {}) {
	return new Response(
		JSON.stringify({
			base_revision: "base-revision",
			candidate_revision: "candidate-revision",
			has_changes: previewOverrides.hasChanges ?? true,
			diff:
				previewOverrides.diff ??
				'-device_name = "relay-one"\n+device_name = "relay-two"\n password = "old-password"',
			check: {
				passed: previewOverrides.passed ?? true,
				message: previewOverrides.message ?? null,
			},
			requires_restart: true,
			password_change_pending: previewOverrides.passwordChange ?? false,
			warnings: previewOverrides.warnings ?? [],
		}),
		{ status: 200, headers: { "Content-Type": "application/json" } },
	);
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((resolvePromise) => {
		resolve = resolvePromise;
	});
	return { promise, resolve };
}

function installApi(
	previewOverrides: PreviewOverrides = {},
	nextPreview?: () => Promise<Response>,
) {
	const requests: Array<{ url: string; init?: RequestInit }> = [];
	const fetchMock = vi.fn(
		async (input: RequestInfo | URL, init?: RequestInit) => {
			const url = String(input);
			requests.push({ url, init });
			if (url === "/api/config" && (!init?.method || init.method === "GET")) {
				return new Response(JSON.stringify(baseConfig), {
					status: 200,
					headers: {
						"Content-Type": "application/json",
						ETag: '"base-revision"',
						"X-Config-Restart-Required": "false",
					},
				});
			}
			if (url === "/api/config/check") {
				return new Response(JSON.stringify({ ok: true }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				});
			}
			if (url === "/api/config/preview") {
				return nextPreview ? nextPreview() : previewResponse(previewOverrides);
			}
			if (url.startsWith("/api/config") && init?.method === "PUT") {
				return new Response(
					JSON.stringify({
						revision: "saved-revision",
						requires_restart: true,
						restart_scheduled: url.includes("restart_after_save=true"),
						session_invalidated: url.includes("restart_after_save=true"),
					}),
					{ status: 200, headers: { "Content-Type": "application/json" } },
				);
			}
			if (url === "/api/service/restart") {
				return new Response(null, { status: 202 });
			}
			throw new Error(`Unexpected request: ${url}`);
		},
	);
	vi.stubGlobal("fetch", fetchMock);
	return { fetchMock, requests };
}

function EditorHarness({
	initialSection = "device",
	setAuth = vi.fn(),
}: {
	initialSection?: ConfigSection;
	setAuth?: (auth: { authenticated: boolean }) => void;
}) {
	const [section, setSection] = useState<ConfigSection>(initialSection);
	return (
		<AuthContext.Provider value={{ auth: { authenticated: true }, setAuth }}>
			<div style={{ height: 900 }}>
				<ConfigEditor section={section} onSectionChange={setSection} />
			</div>
		</AuthContext.Provider>
	);
}

function ResettableFieldsHarness() {
	const [config, setConfig] = useState(() => structuredClone(baseConfig));

	function updatePath(path: string, value: unknown) {
		setConfig((current) => {
			const next = structuredClone(current);
			const keys = path.split(".");
			let target = next as unknown as Record<string, unknown>;
			for (const key of keys.slice(0, -1)) {
				target = target[key] as Record<string, unknown>;
			}
			target[keys.at(-1) as string] = value;
			return next;
		});
	}

	return (
		<>
			<button
				type="button"
				onClick={() => setConfig(structuredClone(baseConfig))}
			>
				Reset
			</button>
			<ConfigSectionEditor
				section="sms"
				config={config}
				onConfigChange={setConfig}
				onPathChange={updatePath}
			/>
			<ConfigSectionEditor
				section="forwarding"
				config={config}
				onConfigChange={setConfig}
				onPathChange={updatePath}
			/>
		</>
	);
}

beforeEach(() => {
	routerMocks.navigate.mockReset();
	routerMocks.shouldBlockFn.mockReset();
	routerMocks.blockerStatus = "idle";
	routerMocks.proceed.mockReset();
	routerMocks.reset.mockReset();
});

afterEach(() => {
	cleanup();
	vi.unstubAllGlobals();
});

describe("ConfigEditor workspace", () => {
	test("associates field help with its form control", async () => {
		installApi();
		render(<EditorHarness />);

		const input = await screen.findByLabelText("Device name");
		const descriptionId = input.getAttribute("aria-describedby");
		expect(descriptionId).toBe("app-device-name-description");
		expect(document.getElementById(descriptionId ?? "")?.textContent).toContain(
			"Included in forwarding payloads",
		);
	});

	test("keeps comma-separated input editable and updates the protected draft", async () => {
		installApi();
		render(<EditorHarness initialSection="sms" />);

		const keywords = (await screen.findByLabelText(
			"Code keywords",
		)) as HTMLInputElement;
		fireEvent.change(keywords, { target: { value: "code," } });
		expect(keywords.value).toBe("code,");
		fireEvent.blur(keywords);
		expect(keywords.value).toBe("code");

		fireEvent.change(keywords, { target: { value: "code, otp" } });
		expect(keywords.value).toBe("code, otp");
		expect(
			routerMocks.shouldBlockFn({
				current: { pathname: "/config" },
				next: { pathname: "/" },
			}),
		).toBe(true);

		fireEvent.blur(keywords);
		expect(keywords.value).toBe("code, otp");
	});

	test("allows replacing a number without writing zero while it is empty", async () => {
		const { requests } = installApi();
		render(<EditorHarness initialSection="forwarding" />);

		const concurrency = (await screen.findByLabelText(
			"Concurrent deliveries",
		)) as HTMLInputElement;
		fireEvent.change(concurrency, { target: { value: "" } });
		expect(concurrency.value).toBe("");
		expect(
			(screen.getByRole("button", { name: "Save" }) as HTMLButtonElement)
				.disabled,
		).toBe(true);

		fireEvent.change(concurrency, { target: { value: "4" } });
		fireEvent.click(screen.getByRole("button", { name: "Check" }));
		await screen.findByText("Check passed");
		const checkRequest = requests.find(
			(request) => request.url === "/api/config/check",
		);
		expect(
			(JSON.parse(checkRequest?.init?.body as string) as AppConfig).delivery
				.concurrency,
		).toBe(4);
	});

	test("reset refreshes mounted array and number input text", () => {
		render(<ResettableFieldsHarness />);

		const keywords = screen.getByLabelText("Code keywords") as HTMLInputElement;
		const concurrency = screen.getByLabelText(
			"Concurrent deliveries",
		) as HTMLInputElement;
		fireEvent.change(keywords, { target: { value: "code, otp" } });
		fireEvent.change(concurrency, { target: { value: "4" } });
		expect(keywords.value).toBe("code, otp");
		expect(concurrency.value).toBe("4");

		fireEvent.click(screen.getByRole("button", { name: "Reset" }));
		expect(keywords.value).toBe("code");
		expect(concurrency.value).toBe("2");
	});

	test("preserves one draft across categories and submits the complete candidate", async () => {
		const { requests } = installApi();
		render(<EditorHarness />);

		const deviceName = await screen.findByLabelText("Device name");
		fireEvent.change(deviceName, { target: { value: "relay-two" } });
		fireEvent.click(screen.getByRole("button", { name: /^SMS/ }));
		fireEvent.change(screen.getByLabelText("Code keywords"), {
			target: { value: "code, otp" },
		});
		expect(screen.getByText("2 categories changed")).toBeTruthy();

		fireEvent.click(screen.getByRole("button", { name: /^Device/ }));
		expect(
			(screen.getByLabelText("Device name") as HTMLInputElement).value,
		).toBe("relay-two");
		fireEvent.click(screen.getByRole("button", { name: "Check" }));
		await screen.findByText("Check passed");

		const checkRequest = requests.find(
			(request) => request.url === "/api/config/check",
		);
		expect(checkRequest).toBeDefined();
		const submitted = JSON.parse(
			checkRequest?.init?.body as string,
		) as AppConfig;
		expect(submitted.app.device_name).toBe("relay-two");
		expect(submitted.sms.code_keywords).toEqual(["code", "otp"]);
		expect(submitted.delivery).toEqual({ concurrency: 2 });
		expect(submitted.http.request_timeout_secs).toBe(30);
		expect(submitted.retention.max_age_days).toBe(90);

		fireEvent.change(screen.getByLabelText("Device name"), {
			target: { value: "relay-three" },
		});
		expect(screen.getByText("Not checked")).toBeTruthy();
	});

	test("previews before PUT and saves the exact captured snapshot", async () => {
		const { requests } = installApi();
		render(<EditorHarness />);

		fireEvent.change(await screen.findByLabelText("Device name"), {
			target: { value: "relay-two" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Save" }));

		const diff = await screen.findByRole("textbox", {
			name: "TOML diff",
		});
		expect((diff as HTMLTextAreaElement).value).toContain("old-password");
		expect(requests.some((request) => request.init?.method === "PUT")).toBe(
			false,
		);
		fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));

		await waitFor(() => {
			expect(requests.some((request) => request.init?.method === "PUT")).toBe(
				true,
			);
		});
		const saveRequest = requests.find(
			(request) => request.init?.method === "PUT",
		);
		expect(saveRequest?.url).toBe("/api/config");
		expect(new Headers(saveRequest?.init?.headers).get("if-match")).toBe(
			"base-revision",
		);
		expect(
			new Headers(saveRequest?.init?.headers).get(
				"x-config-candidate-revision",
			),
		).toBe("candidate-revision");
		const saved = JSON.parse(saveRequest?.init?.body as string) as AppConfig;
		expect(saved.app.device_name).toBe("relay-two");
		await screen.findByText("Configuration saved. Restart required.");
	});

	test("keeps the preview closed when a canceled request finishes", async () => {
		const pendingPreview = deferred<Response>();
		installApi({}, () => pendingPreview.promise);
		render(<EditorHarness />);

		fireEvent.change(await screen.findByLabelText("Device name"), {
			target: { value: "relay-two" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		await screen.findByText("Generating TOML diff and checking the draft…");
		fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

		await act(async () => {
			pendingPreview.resolve(previewResponse());
			await pendingPreview.promise;
		});
		expect(screen.queryByText("Review configuration changes")).toBeNull();
	});

	test("ignores an older preview response after a newer preview is ready", async () => {
		const firstPreview = deferred<Response>();
		const secondPreview = deferred<Response>();
		const pendingPreviews = [firstPreview.promise, secondPreview.promise];
		const { requests } = installApi({}, () => {
			const response = pendingPreviews.shift();
			if (!response) throw new Error("Unexpected preview request");
			return response;
		});
		render(<EditorHarness />);

		const deviceName = await screen.findByLabelText("Device name");
		fireEvent.change(deviceName, { target: { value: "relay-two" } });
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		await screen.findByText("Generating TOML diff and checking the draft…");
		fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

		fireEvent.change(deviceName, { target: { value: "relay-three" } });
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		secondPreview.resolve(
			previewResponse({
				diff: '-device_name = "relay-one"\n+device_name = "relay-three"',
			}),
		);
		const diff = await screen.findByRole("textbox", {
			name: "TOML diff",
		});
		expect((diff as HTMLTextAreaElement).value).toContain("relay-three");

		await act(async () => {
			firstPreview.resolve(
				previewResponse({
					diff: '-device_name = "relay-one"\n+device_name = "relay-two"',
				}),
			);
			await firstPreview.promise;
		});
		expect(
			(
				screen.getByRole("textbox", {
					name: "TOML diff",
				}) as HTMLTextAreaElement
			).value,
		).toContain("relay-three");
		fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));

		await waitFor(() => {
			const saveRequest = requests.find(
				(request) => request.init?.method === "PUT",
			);
			expect(
				JSON.parse(saveRequest?.init?.body as string).app.device_name,
			).toBe("relay-three");
		});
	});

	test("blocks confirmation when the preview check fails", async () => {
		installApi({
			passed: false,
			message: "delivery.concurrency must be between 1 and 16",
		});
		render(<EditorHarness initialSection="forwarding" />);

		fireEvent.change(await screen.findByLabelText("Concurrent deliveries"), {
			target: { value: "0" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Save" }));

		await screen.findByText("Check failed");
		expect(
			screen.getByText("delivery.concurrency must be between 1 and 16"),
		).toBeTruthy();
		const confirm = screen.getByRole("button", { name: "Save configuration" });
		expect((confirm as HTMLButtonElement).disabled).toBe(true);
	});

	test("shows a failed check instead of an earlier save message", async () => {
		const { fetchMock } = installApi();
		render(<EditorHarness />);

		const deviceName = await screen.findByLabelText("Device name");
		fireEvent.change(deviceName, { target: { value: "relay-two" } });
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		fireEvent.click(
			await screen.findByRole("button", { name: "Save configuration" }),
		);
		await screen.findByText("Configuration saved. Restart required.");

		fetchMock.mockImplementationOnce(async () => {
			throw new Error("invalid device name");
		});
		fireEvent.change(deviceName, { target: { value: "relay-three" } });
		expect(screen.getByText("Not checked")).toBeTruthy();
		fireEvent.click(screen.getByRole("button", { name: "Check" }));
		await screen.findByText("Check failed: invalid device name");
	});

	test("uses combined save and restart for a password change then signs out", async () => {
		const setAuth = vi.fn();
		const leavingConfig = {
			current: { pathname: "/config" },
			next: { pathname: "/login" },
		};
		const { requests } = installApi({
			passwordChange: true,
			warnings: ["password_change"],
		});
		render(<EditorHarness initialSection="api" setAuth={setAuth} />);

		fireEvent.change(await screen.findByLabelText("Password"), {
			target: { value: "new-password" },
		});
		expect(routerMocks.shouldBlockFn(leavingConfig)).toBe(true);
		routerMocks.navigate.mockImplementation(async () => {
			expect(routerMocks.shouldBlockFn(leavingConfig)).toBe(false);
		});
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		fireEvent.click(
			await screen.findByRole("button", { name: "Save and schedule restart" }),
		);

		await waitFor(() =>
			expect(setAuth).toHaveBeenCalledWith({ authenticated: false }),
		);
		const saveRequest = requests.find(
			(request) => request.init?.method === "PUT",
		);
		expect(saveRequest?.url).toBe("/api/config?restart_after_save=true");
		expect(
			requests.some((request) => request.url === "/api/service/restart"),
		).toBe(false);
		expect(routerMocks.navigate).toHaveBeenCalledWith(
			expect.objectContaining({
				to: "/login",
				search: { notice: "config_saved_restart_scheduled" },
			}),
		);
	});

	test("uses singular category copy for one changed section", async () => {
		installApi();
		render(<EditorHarness />);

		fireEvent.change(await screen.findByLabelText("Device name"), {
			target: { value: "relay-two" },
		});
		expect(screen.getByText("1 category changed")).toBeTruthy();
	});

	test("shows an unknown operational warning code instead of a blank row", async () => {
		installApi({ warnings: ["future_warning"] });
		render(<EditorHarness />);

		fireEvent.change(await screen.findByLabelText("Device name"), {
			target: { value: "relay-two" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		expect(await screen.findByText("future_warning")).toBeTruthy();
	});

	test.each([
		["Stay", "reset"],
		["Discard and leave", "proceed"],
	] as const)("resolves blocked navigation with %s", async (label, action) => {
		installApi();
		render(<EditorHarness />);

		const deviceName = await screen.findByLabelText("Device name");
		routerMocks.blockerStatus = "blocked";
		fireEvent.change(deviceName, { target: { value: "relay-two" } });
		expect(await screen.findByText("Leave with unsaved changes?")).toBeTruthy();
		fireEvent.click(screen.getByRole("button", { name: label }));
		expect(routerMocks[action]).toHaveBeenCalledOnce();
	});

	test("exposes delivery, timeout, and retention categories", async () => {
		installApi();
		render(<EditorHarness initialSection="forwarding" />);

		await screen.findByLabelText("Concurrent deliveries");
		fireEvent.click(screen.getByRole("button", { name: /^Timeouts/ }));
		expect(screen.getByLabelText("Connect timeout")).toBeTruthy();
		fireEvent.click(screen.getByRole("button", { name: /^Retention/ }));
		expect(screen.getByLabelText("Maximum age")).toBeTruthy();
	});
});
