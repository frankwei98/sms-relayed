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
	sms_over_ims: {
		status: "available" as const,
		support: "supported" as const,
		configured: "enabled" as const,
		registration: "registered" as const,
		sms_service: "available" as const,
		technology: "wlan" as const,
		probe: {
			tool: "qmicli",
			available: true,
			version_raw: "qmicli 1.36.0",
			transport: "direct_qmi" as const,
			device: "/dev/wwan0qmi0",
			capabilities: {
				ims_settings: true,
				imsa_registration: true,
				imsa_services: true,
			},
		},
		evidence: [
			"qmi_ims_settings",
			"qmi_imsa_registration",
			"qmi_imsa_services",
		],
		reasons: [],
		warnings: [],
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

describe("ModemStatusPanel SMS over IMS", () => {
	test("shows available IMS SMS over WLAN with its evidence", async () => {
		mocks.fetchModemStatus.mockResolvedValue(status);

		render(<ModemStatusPanel />);

		expect(await screen.findByText("SMS over IMS")).toBeTruthy();
		expect(screen.getByText("Available over WLAN")).toBeTruthy();
		expect(screen.getByText("QMI IMSA · WLAN")).toBeTruthy();
	});

	test.each([
		["available", "wwan", "Available"],
		["registering", "unknown", "Registering"],
		["limited", "wwan", "Limited"],
		["not_registered", "unknown", "Not Registered"],
		["unavailable", "unknown", "Unavailable"],
		["unknown", "unknown", "Unknown"],
	] as const)("shows %s as %s", async (imsStatus, technology, label) => {
		mocks.fetchModemStatus.mockResolvedValue({
			...status,
			sms_over_ims: {
				...status.sms_over_ims,
				status: imsStatus,
				technology,
			},
		});

		render(<ModemStatusPanel />);

		expect((await screen.findAllByText(label)).length).toBeGreaterThan(0);
	});

	test("explains nonstandard modem output without exposing its reason code", async () => {
		mocks.fetchModemStatus.mockResolvedValue({
			...status,
			sms_over_ims: {
				...status.sms_over_ims,
				warnings: ["ims_services_output_nonstandard"],
			},
		});

		render(<ModemStatusPanel />);

		expect(
			await screen.findByText(
				"The IMS service response used a nonstandard label.",
			),
		).toBeTruthy();
		expect(screen.queryByText("ims_services_output_nonstandard")).toBeNull();
	});
});
