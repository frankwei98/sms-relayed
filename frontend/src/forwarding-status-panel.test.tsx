// @vitest-environment jsdom

import {
	act,
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import { StrictMode, useState } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ForwardingStatusPanel } from "#/components/forwarding/forwarding-status-panel";
import type { ForwardAttemptSample, ProfileStatus } from "#/lib/api";

const mocks = vi.hoisted(() => ({
	apiFetch: vi.fn(),
}));

vi.mock("#/lib/api", () => ({ apiFetch: mocks.apiFetch }));

const successSample = {
	attempt_number: 2,
	is_retry: true,
	started_at: "2026-07-12T17:00:00Z",
	completed_at: "2026-07-12T17:00:01Z",
	latency_ms: 950,
	dispatch_delay_ms: 18,
	outcome: "success" as const,
	error_code: null,
};

const transientFailureSample = {
	attempt_number: 1,
	is_retry: false,
	started_at: "2026-07-12T17:01:00Z",
	completed_at: "2026-07-12T17:01:05Z",
	latency_ms: 5200,
	dispatch_delay_ms: null,
	outcome: "transient_failure" as const,
	error_code: "http_timeout",
};

const permanentFailureSample = {
	attempt_number: 3,
	is_retry: false,
	started_at: "2026-07-12T17:03:00Z",
	completed_at: "2026-07-12T17:03:01Z",
	latency_ms: 1100,
	dispatch_delay_ms: 4,
	outcome: "permanent_failure" as const,
	error_code: "invalid_target",
};

const legacySample = {
	...successSample,
	dispatch_delay_ms: undefined,
};

type TestSample = Omit<ForwardAttemptSample, "dispatch_delay_ms"> & {
	dispatch_delay_ms?: number | null;
};

type TestProfile = Omit<ProfileStatus, "samples"> & {
	samples: TestSample[];
};

function forwardingResponse(
	profiles: TestProfile[],
	overrides: Partial<{ generated_at: string; sample_limit: number }> = {},
) {
	return {
		generated_at: "2026-07-12T17:02:00Z",
		sample_limit: 5,
		profiles,
		...overrides,
	};
}

function configuredProfile(
	profileKey: string,
	options: Partial<Pick<TestProfile, "enabled" | "samples">> = {},
): TestProfile {
	return {
		profile_key: profileKey,
		configured: true,
		enabled: options.enabled ?? true,
		samples: options.samples ?? [],
	};
}

function historicalProfile(
	profileKey: string,
	samples: TestProfile["samples"],
): TestProfile {
	return {
		profile_key: profileKey,
		configured: false,
		enabled: false,
		samples,
	};
}

function ControlledPanel({
	initialProfile,
	onSelection,
}: {
	initialProfile?: string;
	onSelection?: (profile?: string) => void;
}) {
	const [selectedProfile, setSelectedProfile] = useState(initialProfile);
	return (
		<ForwardingStatusPanel
			selectedProfile={selectedProfile}
			onSelectProfile={(profile) => {
				onSelection?.(profile);
				setSelectedProfile(profile);
			}}
		/>
	);
}

function contentRefreshButton() {
	const button = screen
		.getAllByRole("button", { name: "Refresh forwarding status" })
		.find((candidate) => candidate.textContent?.includes("Refresh"));
	expect(button).toBeDefined();
	if (!button) throw new Error("Content refresh button was not rendered");
	return button;
}

function expectMetric(label: string, value: number) {
	const labelElement = screen.getByText(label);
	expect(labelElement.parentElement?.textContent).toContain(String(value));
}

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe("ForwardingStatusPanel", () => {
	test("renders truthful overview metrics and configured and historical navigation", async () => {
		mocks.apiFetch.mockResolvedValue(
			forwardingResponse([
				configuredProfile("bark.primary", { samples: [successSample] }),
				configuredProfile("telegram.backup", { enabled: false }),
				historicalProfile("legacy.webhook", [transientFailureSample]),
			]),
		);

		render(<ControlledPanel />);

		expect(screen.getByText("Loading forwarding status...")).toBeDefined();

		await waitFor(() => {
			expect(screen.getByText("Forwarding coverage")).toBeDefined();
		});

		expectMetric("Configured profiles", 2);
		expectMetric("Enabled profiles", 1);
		expectMetric("Profiles with retained attempts", 2);

		const navigation = screen.getByRole("complementary", {
			name: "Forwarding profiles",
		});
		const overviewButton = within(navigation).getByRole("button", {
			name: /Overview/,
		});
		expect(overviewButton.getAttribute("aria-current")).toBe("page");
		expect(
			within(navigation).getByRole("heading", { name: "Configured" }),
		).toBeDefined();
		expect(
			within(navigation).getByRole("heading", { name: "Historical" }),
		).toBeDefined();
		expect(within(navigation).getByText("bark.primary")).toBeDefined();
		expect(within(navigation).getByText("telegram.backup")).toBeDefined();
		expect(within(navigation).getByText("legacy.webhook")).toBeDefined();
		expect(within(navigation).getByText("Enabled")).toBeDefined();
		expect(within(navigation).getByText("Disabled")).toBeDefined();
		expect(
			within(navigation).getAllByText("Historical").length,
		).toBeGreaterThan(0);
		expect(within(navigation).getByText("Latest: Success")).toBeDefined();
		expect(within(navigation).getByText("No attempts")).toBeDefined();
		expect(
			within(navigation).getByText("Latest: Transient failure"),
		).toBeDefined();

		const pageText = document.body.textContent?.toLowerCase() ?? "";
		expect(pageText).not.toContain("success rate");
		expect(pageText).not.toContain("health threshold");
		expect(pageText).not.toContain("trend");
		expect(pageText).not.toContain("queue depth");
	});

	test("shows selected profile state, latest result, and responsive attempt records", async () => {
		mocks.apiFetch.mockResolvedValue(
			forwardingResponse([
				configuredProfile("bark.primary", {
					samples: [successSample, transientFailureSample],
				}),
			]),
		);

		render(<ControlledPanel initialProfile="bark.primary" />);

		await waitFor(() => {
			expect(screen.getByText("Latest 5 attempts")).toBeDefined();
		});

		expect(screen.getAllByText("Configured").length).toBeGreaterThan(0);
		expect(screen.getAllByText("Enabled").length).toBeGreaterThan(0);
		expect(screen.getAllByText("Success").length).toBeGreaterThan(0);
		expect(screen.getAllByText("Transient failure").length).toBeGreaterThan(0);
		expect(screen.getByText("2 retained in this snapshot")).toBeDefined();
		expect(
			document.querySelectorAll('time[datetime="2026-07-12T17:00:01Z"]').length,
		).toBeGreaterThan(0);

		const table = screen.getByRole("table", {
			name: "Latest 5 attempts for bark.primary",
		});
		expect(within(table).getByText("Attempt")).toBeDefined();
		expect(within(table).getByText("Completed")).toBeDefined();
		expect(within(table).queryByText("Started")).toBeNull();
		expect(within(table).getByText("Retry")).toBeDefined();
		expect(
			within(table).getByText("Dispatch 18ms · Request 950ms"),
		).toBeDefined();
		expect(within(table).getByText("Dispatch — · Request 5.2s")).toBeDefined();
		expect(within(table).getByText("http_timeout")).toBeDefined();

		const mobileRecords = screen.getByRole("list", {
			name: "Latest 5 attempts for bark.primary",
		});
		expect(within(mobileRecords).getAllByText("Completed").length).toBe(2);
		expect(within(mobileRecords).getAllByText("Timing").length).toBe(2);
		expect(within(mobileRecords).getAllByText("Error").length).toBe(2);
	});

	test("supports opaque profile keys and closes mobile navigation after selection", async () => {
		const profileKey = "ops/profile?region=east#primary [v2]";
		const onSelection = vi.fn();
		mocks.apiFetch.mockResolvedValue(
			forwardingResponse([
				configuredProfile(profileKey, { samples: [permanentFailureSample] }),
			]),
		);

		render(<ControlledPanel onSelection={onSelection} />);

		const navigation = await screen.findByRole("complementary", {
			name: "Forwarding profiles",
		});
		const openNavigation = screen.getByRole("button", {
			name: "Open forwarding navigation",
		});
		expect(openNavigation.getAttribute("aria-expanded")).toBe("true");
		const keyElement = within(navigation).getByText(profileKey);
		const profileButton = keyElement.closest("button");
		expect(profileButton).not.toBeNull();
		if (!profileButton)
			throw new Error("Profile navigation row was not rendered");

		fireEvent.click(profileButton);

		await waitFor(() => {
			expect(onSelection).toHaveBeenCalledWith(profileKey);
			expect(profileButton.getAttribute("aria-current")).toBe("page");
			expect(openNavigation.getAttribute("aria-expanded")).toBe("false");
		});
		expect(screen.getAllByText(profileKey).length).toBeGreaterThan(0);
		expect(screen.getAllByText("Permanent failure").length).toBeGreaterThan(0);

		fireEvent.click(openNavigation);
		expect(openNavigation.getAttribute("aria-expanded")).toBe("true");
		const closeNavigation = screen.getByRole("button", {
			name: "Close forwarding navigation",
		});
		expect(closeNavigation.getAttribute("aria-expanded")).toBe("true");
		fireEvent.click(closeNavigation);
		expect(openNavigation.getAttribute("aria-expanded")).toBe("false");
	});

	test("shows no-attempt state without inventing an outcome", async () => {
		mocks.apiFetch.mockResolvedValue(
			forwardingResponse([configuredProfile("quiet.profile")], {
				sample_limit: 1,
			}),
		);

		render(<ControlledPanel initialProfile="quiet.profile" />);

		await waitFor(() => {
			expect(screen.getByText("Latest 1 attempt")).toBeDefined();
		});
		expect(screen.getByText("No forwarding attempts yet.")).toBeDefined();
		expect(screen.getAllByText("No attempts").length).toBeGreaterThan(0);
		expect(screen.queryByText("Success")).toBeNull();
		expect(screen.queryByText("Transient failure")).toBeNull();
		expect(screen.queryByText("Permanent failure")).toBeNull();
	});

	test("shows unknown dispatch timing when an older response omits the field", async () => {
		mocks.apiFetch.mockResolvedValue(
			forwardingResponse([
				{
					...configuredProfile("bark.primary"),
					samples: [legacySample],
				},
			]),
		);

		render(<ControlledPanel initialProfile="bark.primary" />);

		await waitFor(() => {
			expect(
				screen.getAllByText("Dispatch — · Request 950ms").length,
			).toBeGreaterThan(0);
		});
	});

	test("renders zero metrics and the empty snapshot state when there are no profiles", async () => {
		mocks.apiFetch.mockResolvedValue(forwardingResponse([]));

		render(<ControlledPanel />);

		await waitFor(() => {
			expect(
				screen.getByText("No forwarding profiles configured."),
			).toBeDefined();
		});
		expectMetric("Configured profiles", 0);
		expectMetric("Enabled profiles", 0);
		expectMetric("Profiles with retained attempts", 0);
		expect(screen.getByText("No configured profiles")).toBeDefined();
		expect(screen.getByText("No retained historical profiles")).toBeDefined();
	});

	test("shows an initial error and can retry the load", async () => {
		mocks.apiFetch.mockRejectedValueOnce(new Error("network error"));
		mocks.apiFetch.mockResolvedValueOnce(
			forwardingResponse([
				configuredProfile("recovered.profile", { samples: [successSample] }),
			]),
		);

		render(
			<ForwardingStatusPanel
				onSelectProfile={vi.fn()}
				selectedProfile={undefined}
			/>,
		);

		await waitFor(() => {
			expect(screen.getByText("network error")).toBeDefined();
		});
		expect(screen.getByText("Unable to load forwarding status")).toBeDefined();

		fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

		await waitFor(() => {
			expect(screen.getAllByText("recovered.profile").length).toBeGreaterThan(
				0,
			);
		});
		expect(mocks.apiFetch).toHaveBeenCalledTimes(2);
	});

	test("manual refresh disables controls, marks the workspace busy, and updates data", async () => {
		mocks.apiFetch.mockResolvedValueOnce(forwardingResponse([]));
		let resolveRefresh: (value: unknown) => void = () => {};
		mocks.apiFetch.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					resolveRefresh = resolve;
				}),
		);

		const { container } = render(<ControlledPanel />);
		await waitFor(() => {
			expect(
				screen.getByText("No forwarding profiles configured."),
			).toBeDefined();
		});

		const refreshButton = contentRefreshButton();
		fireEvent.click(refreshButton);

		expect(refreshButton.hasAttribute("disabled")).toBe(true);
		expect(refreshButton.querySelector(".animate-spin")).not.toBeNull();
		expect(container.querySelector('[aria-busy="true"]')).not.toBeNull();
		expect(screen.getByText("Refreshing forwarding status.")).toBeDefined();

		await act(async () => {
			resolveRefresh(
				forwardingResponse([
					configuredProfile("bark.primary", { samples: [successSample] }),
				]),
			);
		});

		await waitFor(() => {
			expect(screen.getAllByText("bark.primary").length).toBeGreaterThan(0);
			expect(refreshButton.hasAttribute("disabled")).toBe(false);
		});
		expect(refreshButton.querySelector(".animate-spin")).toBeNull();
		expect(mocks.apiFetch).toHaveBeenNthCalledWith(
			2,
			"/api/forwarding/attempts",
		);
	});

	test("keeps the previous snapshot visible when manual refresh fails", async () => {
		mocks.apiFetch.mockResolvedValueOnce(
			forwardingResponse([
				configuredProfile("bark.primary", { samples: [successSample] }),
			]),
		);
		mocks.apiFetch.mockRejectedValueOnce(new Error("refresh network error"));

		render(<ControlledPanel initialProfile="bark.primary" />);
		await screen.findByText("Latest 5 attempts");

		const refreshButton = contentRefreshButton();
		fireEvent.click(refreshButton);

		await waitFor(() => {
			expect(screen.getAllByText("Refresh failed").length).toBeGreaterThan(0);
		});
		expect(
			screen.getAllByText(
				"Showing the previous snapshot. refresh network error",
			).length,
		).toBeGreaterThan(0);
		expect(screen.getAllByText("bark.primary").length).toBeGreaterThan(0);
		expect(screen.getByText("Latest 5 attempts")).toBeDefined();
		expect(refreshButton.hasAttribute("disabled")).toBe(false);
	});

	test("ignores a stale initial response when a newer generation finishes first", async () => {
		let resolveFirst: (value: unknown) => void = () => {};
		let resolveSecond: (value: unknown) => void = () => {};
		mocks.apiFetch
			.mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						resolveFirst = resolve;
					}),
			)
			.mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						resolveSecond = resolve;
					}),
			);

		render(
			<StrictMode>
				<ForwardingStatusPanel
					selectedProfile={undefined}
					onSelectProfile={vi.fn()}
				/>
			</StrictMode>,
		);

		await waitFor(() => {
			expect(mocks.apiFetch).toHaveBeenCalledTimes(2);
		});

		await act(async () => {
			resolveSecond(
				forwardingResponse([
					configuredProfile("new.snapshot", { samples: [successSample] }),
				]),
			);
		});
		await screen.findAllByText("new.snapshot");

		await act(async () => {
			resolveFirst(
				forwardingResponse([
					configuredProfile("stale.snapshot", {
						samples: [transientFailureSample],
					}),
				]),
			);
		});

		expect(screen.getAllByText("new.snapshot").length).toBeGreaterThan(0);
		expect(screen.queryByText("stale.snapshot")).toBeNull();
	});

	test("keeps a missing controlled selection in a stable unavailable state", async () => {
		const onSelection = vi.fn();
		mocks.apiFetch.mockResolvedValueOnce(
			forwardingResponse([
				configuredProfile("removed/profile", { samples: [successSample] }),
			]),
		);
		mocks.apiFetch.mockResolvedValueOnce(forwardingResponse([]));

		render(
			<ControlledPanel
				initialProfile="removed/profile"
				onSelection={onSelection}
			/>,
		);
		await screen.findByText("Latest 5 attempts");

		fireEvent.click(contentRefreshButton());

		await waitFor(() => {
			expect(screen.getByText("Profile unavailable")).toBeDefined();
		});
		expect(screen.getAllByText("removed/profile").length).toBeGreaterThan(0);
		expect(
			screen.getByText("is not present in the latest forwarding snapshot.", {
				exact: false,
			}),
		).toBeDefined();

		fireEvent.click(screen.getByRole("button", { name: "View Overview" }));

		await waitFor(() => {
			expect(onSelection).toHaveBeenLastCalledWith(undefined);
			expect(screen.getByText("Forwarding coverage")).toBeDefined();
		});
	});

	test("does not render phone numbers, SMS bodies, or secret payload fields", async () => {
		const sampleWithPrivateExtras = {
			...successSample,
			error_code: "shell_exit_nonzero",
			phone_number: "+15551234567",
			body: "private sms body",
			token: "super-secret-token",
		};
		mocks.apiFetch.mockResolvedValue(
			forwardingResponse([
				{
					...configuredProfile("bark.primary"),
					samples: [sampleWithPrivateExtras],
				},
			]),
		);

		render(<ControlledPanel initialProfile="bark.primary" />);

		await waitFor(() => {
			expect(screen.getAllByText("bark.primary").length).toBeGreaterThan(0);
		});
		const html = document.body.innerHTML;
		expect(html).not.toContain("+15551234567");
		expect(html).not.toContain("private sms body");
		expect(html).not.toContain("super-secret-token");
		expect(html).toContain("shell_exit_nonzero");
	});
});
