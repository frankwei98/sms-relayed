// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import {
	ChannelEditor,
	normalizeForwardEnabled,
} from "#/components/config/channel-editor";
import type { AppConfig } from "#/lib/config-model";

const config: AppConfig = {
	app: { device_name: "router", modem_path: "/modem/0" },
	sms: { ignore_storage: [], code_keywords: [] },
	forward: { enabled: ["telegram.alerts", "bark.missing"] },
	channels: {
		bark: {
			primary: { server_url: "https://api.day.app", key: "secret" },
		},
		telegram: {
			alerts: {
				bot_token: "token",
				chat_id: "chat",
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
		password: "password",
		database_path: "/tmp/sms-relayed.sqlite",
	},
	http: {
		connect_timeout_secs: 5,
		request_timeout_secs: 10,
		shell_timeout_secs: 30,
	},
	retention: { enabled: true, max_age_days: 30, batch_size: 100 },
};

afterEach(cleanup);

describe("ChannelEditor forwarding controls", () => {
	test("drops enabled refs that do not match an existing profile", () => {
		expect(normalizeForwardEnabled(config).forward.enabled).toEqual([
			"telegram.alerts",
		]);
	});

	test("aggregates profile switches into forward.enabled", () => {
		const onUpdate = vi.fn();
		const view = render(<ChannelEditor config={config} onUpdate={onUpdate} />);

		fireEvent.click(
			screen.getByRole("switch", {
				name: "Enable forwarding for bark.primary",
			}),
		);

		const enabledConfig = onUpdate.mock.calls[0][0] as AppConfig;
		expect(enabledConfig.forward.enabled).toEqual([
			"telegram.alerts",
			"bark.primary",
		]);

		view.rerender(<ChannelEditor config={enabledConfig} onUpdate={onUpdate} />);
		fireEvent.click(
			screen.getByRole("switch", {
				name: "Enable forwarding for telegram.alerts",
			}),
		);

		const disabledConfig = onUpdate.mock.calls[1][0] as AppConfig;
		expect(disabledConfig.forward.enabled).toEqual(["bark.primary"]);
	});
});
