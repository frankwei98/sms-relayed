// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ModemStatusPanel } from "#/components/modem/modem-status-panel";

const mocks = vi.hoisted(() => ({
	fetchModemStatus: vi.fn(),
	runModemAction: vi.fn(),
}));

vi.mock("#/lib/modem-api", () => ({
	fetchModemStatus: mocks.fetchModemStatus,
	runModemAction: mocks.runModemAction,
}));

const status = {
	checked_at: "2026-07-25T12:00:00Z",
	tool: {
		available: true,
		version_raw: "mmcli 1.22.0",
		supports_json: true,
	},
	configured_modem_path: "/org/freedesktop/ModemManager1/Modem/0",
	resolved: {
		present: true,
		id: "0",
		path: "/org/freedesktop/ModemManager1/Modem/0",
	},
	health: {
		status: "ok" as const,
		reasons: [],
	},
	modem: {
		enabled: true,
		state: "registered",
		sim_state: "ready",
		own_number: "+6581234567",
		operator_name: "Example Telecom",
		signal_quality: 78,
		access_technologies: ["lte"],
	},
	messaging: {
		available: true,
		supported_storages: ["sm"],
		default_storage: "sm",
	},
	diagnostics: {
		last_error: null,
		path_drift_candidate: null,
	},
};

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe("ModemStatusPanel phone number", () => {
	test("shows the reported number with a copy action", async () => {
		mocks.fetchModemStatus.mockResolvedValue(status);

		render(<ModemStatusPanel />);

		expect(await screen.findByText("+6581234567")).toBeTruthy();
		expect(
			screen.getByRole("button", { name: "Copy phone number" }),
		).toBeTruthy();
	});

	test("shows not reported when the modem has no own number", async () => {
		mocks.fetchModemStatus.mockResolvedValue({
			...status,
			modem: { ...status.modem, own_number: null },
		});

		render(<ModemStatusPanel />);

		expect(await screen.findByText("not reported")).toBeTruthy();
		expect(
			screen.queryByRole("button", { name: "Copy phone number" }),
		).toBeNull();
	});
});
