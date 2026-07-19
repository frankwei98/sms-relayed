// @vitest-environment jsdom

import {
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ForwardingStatusPanel } from "#/components/forwarding/forwarding-status-panel";

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

const failureSample = {
	attempt_number: 1,
	is_retry: false,
	started_at: "2026-07-12T17:01:00Z",
	completed_at: "2026-07-12T17:01:05Z",
	latency_ms: 5200,
	dispatch_delay_ms: null,
	outcome: "transient_failure" as const,
	error_code: "http_timeout",
};

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe("ForwardingStatusPanel", () => {
	test("shows loading then profiles with samples", async () => {
		mocks.apiFetch.mockResolvedValue({
			generated_at: "2026-07-12T17:02:00Z",
			profiles: [
				{
					profile_key: "bark.primary",
					enabled: true,
					samples: [successSample],
				},
				{
					profile_key: "telegram.backup",
					enabled: false,
					samples: [failureSample],
				},
			],
		});

		render(<ForwardingStatusPanel />);

		expect(screen.getByText("Loading forwarding status...")).toBeDefined();

		await waitFor(() => {
			expect(screen.getAllByText("bark.primary").length).toBeGreaterThan(0);
		});
		expect(screen.getAllByText("telegram.backup").length).toBeGreaterThan(0);
		expect(screen.getByText("Retry")).toBeDefined();
		expect(screen.getByText("Dispatch 18ms · Request 950ms")).toBeDefined();
		expect(screen.getByText("Dispatch — · Request 5.2s")).toBeDefined();
		expect(screen.getByText("http_timeout")).toBeDefined();
	});

	test("shows latest outcome badge in profile header", async () => {
		mocks.apiFetch.mockResolvedValue({
			generated_at: "2026-07-12T17:02:00Z",
			profiles: [
				{
					profile_key: "bark.primary",
					enabled: true,
					samples: [successSample, failureSample],
				},
			],
		});

		render(<ForwardingStatusPanel />);

		await waitFor(() => {
			expect(screen.getAllByText("bark.primary").length).toBeGreaterThan(0);
		});
		const successElements = screen.getAllByText("Success");
		expect(successElements.length).toBeGreaterThanOrEqual(1);
	});

	test("no outcome badge when samples are empty", async () => {
		mocks.apiFetch.mockResolvedValue({
			generated_at: "2026-07-12T17:02:00Z",
			profiles: [
				{
					profile_key: "bark.primary",
					enabled: true,
					samples: [],
				},
			],
		});

		render(<ForwardingStatusPanel />);

		await waitFor(() => {
			expect(screen.getAllByText("bark.primary").length).toBeGreaterThan(0);
		});
		expect(screen.getByText("No forwarding attempts yet.")).toBeDefined();
	});

	test("column header reads Completed not Started", async () => {
		mocks.apiFetch.mockResolvedValue({
			generated_at: "2026-07-12T17:02:00Z",
			profiles: [
				{
					profile_key: "bark.primary",
					enabled: true,
					samples: [successSample],
				},
			],
		});

		render(<ForwardingStatusPanel />);

		await waitFor(() => {
			const completedHeaders = screen.getAllByText("Completed");
			expect(completedHeaders.length).toBeGreaterThan(0);
		});
		expect(screen.queryByText("Started")).toBeNull();
	});

	test("renders empty state when no profiles", async () => {
		mocks.apiFetch.mockResolvedValue({
			generated_at: "2026-07-12T17:02:00Z",
			profiles: [],
		});

		render(<ForwardingStatusPanel />);

		await waitFor(() => {
			expect(
				screen.getByText("No forwarding profiles configured."),
			).toBeDefined();
		});
	});

	test("shows error state on API failure", async () => {
		mocks.apiFetch.mockRejectedValue(new Error("network error"));

		render(<ForwardingStatusPanel />);

		await waitFor(() => {
			expect(screen.getByText("network error")).toBeDefined();
		});
	});

	test("manual refresh triggers data update", async () => {
		// Use mockResolvedValue (not Once) since React may call twice in dev
		mocks.apiFetch.mockResolvedValue({
			generated_at: "2026-07-12T17:00:00Z",
			profiles: [],
		});

		render(<ForwardingStatusPanel />);

		await waitFor(() => {
			expect(
				screen.getByText("No forwarding profiles configured."),
			).toBeDefined();
		});

		// Re-mock for the refresh call
		mocks.apiFetch.mockResolvedValue({
			generated_at: "2026-07-12T17:01:00Z",
			profiles: [
				{
					profile_key: "bark.primary",
					enabled: true,
					samples: [successSample],
				},
			],
		});

		const buttons = screen.getAllByRole("button");
		const refreshButton = buttons.find((b) =>
			b.textContent?.includes("Refresh"),
		);
		expect(refreshButton).toBeDefined();

		if (refreshButton) {
			fireEvent.click(refreshButton);
		}

		await waitFor(() => {
			expect(screen.getAllByText("bark.primary").length).toBeGreaterThan(0);
		});
	});

	test("manual refresh disables the button and shows progress until completion", async () => {
		mocks.apiFetch.mockResolvedValueOnce({
			generated_at: "2026-07-12T17:00:00Z",
			profiles: [],
		});
		let resolveRefresh: (value: unknown) => void = () => {};
		mocks.apiFetch.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					resolveRefresh = resolve;
				}),
		);

		render(<ForwardingStatusPanel />);
		await waitFor(() => {
			expect(
				screen.getByText("No forwarding profiles configured."),
			).toBeDefined();
		});

		const refreshButton = screen.getByRole("button", { name: /refresh/i });
		fireEvent.click(refreshButton);
		expect(refreshButton.hasAttribute("disabled")).toBe(true);
		expect(refreshButton.querySelector(".animate-spin")).not.toBeNull();

		resolveRefresh({
			generated_at: "2026-07-12T17:01:00Z",
			profiles: [],
		});
		await waitFor(() => {
			expect(refreshButton.hasAttribute("disabled")).toBe(false);
		});
		expect(refreshButton.querySelector(".animate-spin")).toBeNull();
	});

	test("DOM does not contain phone numbers or SMS bodies", async () => {
		mocks.apiFetch.mockResolvedValue({
			generated_at: "2026-07-12T17:02:00Z",
			profiles: [
				{
					profile_key: "bark.primary",
					enabled: true,
					samples: [
						{
							...successSample,
							error_code: "shell_exit_nonzero",
						},
					],
				},
			],
		});

		render(<ForwardingStatusPanel />);

		await waitFor(() => {
			expect(screen.getAllByText("bark.primary").length).toBeGreaterThan(0);
		});
		const html = document.body.innerHTML;
		expect(html).not.toContain("+1555");
		expect(html).not.toContain("sms body");
		expect(html).not.toContain("token");
		expect(html).toContain("shell_exit_nonzero");
	});
});
