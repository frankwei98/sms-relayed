// @vitest-environment jsdom

import { render, screen, waitFor } from "@testing-library/react";
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
	outcome: "success" as const,
	error_code: null,
};

const failureSample = {
	attempt_number: 1,
	is_retry: false,
	started_at: "2026-07-12T17:01:00Z",
	completed_at: "2026-07-12T17:01:05Z",
	latency_ms: 5200,
	outcome: "transient_failure" as const,
	error_code: "http_timeout",
};

afterEach(() => {
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
			expect(screen.getByText("bark.primary")).toBeDefined();
		});
		expect(screen.getByText("telegram.backup")).toBeDefined();
		expect(screen.getByText("Retry")).toBeDefined();
		expect(screen.getByText("950ms")).toBeDefined();
		expect(screen.getByText("5.2s")).toBeDefined();
		expect(screen.getByText("http_timeout")).toBeDefined();
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
			expect(screen.getByText("bark.primary")).toBeDefined();
		});
		const html = document.body.innerHTML;
		expect(html).not.toContain("+1555");
		expect(html).not.toContain("sms body");
		expect(html).not.toContain("token");
	});
});
