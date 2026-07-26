// @vitest-environment jsdom

import {
	cleanup,
	fireEvent,
	render,
	screen,
	within,
} from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ChannelEditor } from "#/components/config/channel-editor";
import type { AppConfig } from "#/lib/config-model";

const config: AppConfig = {
	app: { device_name: "router", modem_path: "/modem/0" },
	sms: { ignore_storage: [], code_keywords: [] },
	forward: { enabled: ["telegram.alerts", "bark.missing"] },
	delivery: { concurrency: 2 },
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
			"bark.missing",
			"bark.primary",
		]);

		view.rerender(<ChannelEditor config={enabledConfig} onUpdate={onUpdate} />);
		fireEvent.click(
			screen.getByRole("switch", {
				name: "Enable forwarding for telegram.alerts",
			}),
		);

		const disabledConfig = onUpdate.mock.calls[1][0] as AppConfig;
		expect(disabledConfig.forward.enabled).toEqual([
			"bark.missing",
			"bark.primary",
		]);
	});

	test("shows missing profile refs and removes only the selected ref", () => {
		const configWithMissingRefs: AppConfig = {
			...config,
			forward: {
				...config.forward,
				enabled: [
					"telegram.alerts",
					"bark.missing",
					"shell.ghost",
					"bark.missing",
				],
			},
		};
		const onUpdate = vi.fn();
		render(
			<ChannelEditor config={configWithMissingRefs} onUpdate={onUpdate} />,
		);

		const warning = screen.getByRole("region", {
			name: "Missing forwarding profiles",
		});
		expect(within(warning).getAllByText("bark.missing")).toHaveLength(1);
		expect(within(warning).getByText("shell.ghost")).toBeTruthy();

		fireEvent.click(
			within(warning).getByRole("button", {
				name: "Remove missing forwarding reference bark.missing",
			}),
		);

		expect(onUpdate).toHaveBeenCalledWith({
			...configWithMissingRefs,
			forward: {
				...configWithMissingRefs.forward,
				enabled: ["telegram.alerts", "shell.ghost"],
			},
		});
	});

	test("does not overwrite a duplicate profile name", () => {
		const onUpdate = vi.fn();
		render(<ChannelEditor config={config} onUpdate={onUpdate} />);

		fireEvent.change(screen.getByLabelText("Add bark profile"), {
			target: { value: "primary" },
		});

		const add = screen.getAllByRole("button", { name: "Add" })[0];
		const input = screen.getByLabelText("Add bark profile");
		const error = screen.getByText("That profile name already exists.");
		expect((add as HTMLButtonElement).disabled).toBe(true);
		expect(input.getAttribute("aria-describedby")).toBe(error.id);
		expect(onUpdate).not.toHaveBeenCalled();
	});

	test("keeps special characters in opaque profile names", () => {
		const onUpdate = vi.fn();
		render(<ChannelEditor config={config} onUpdate={onUpdate} />);

		fireEvent.change(screen.getByLabelText("Add bark profile"), {
			target: { value: "team/ops.v2" },
		});
		fireEvent.click(screen.getAllByRole("button", { name: "Add" })[0]);

		const updated = onUpdate.mock.calls[0][0] as AppConfig;
		expect(updated.channels.bark["team/ops.v2"]).toEqual({
			server_url: "",
			key: "",
		});
	});

	test("confirms removal and only drops the matching enabled ref", () => {
		const onUpdate = vi.fn();
		render(<ChannelEditor config={config} onUpdate={onUpdate} />);

		fireEvent.click(
			screen.getByRole("button", { name: "Remove bark.primary" }),
		);
		expect(screen.getByText("Remove forwarding profile?")).toBeTruthy();
		fireEvent.click(screen.getByRole("button", { name: "Remove profile" }));

		const updated = onUpdate.mock.calls[0][0] as AppConfig;
		expect(updated.channels.bark.primary).toBeUndefined();
		expect(updated.forward.enabled).toEqual([
			"telegram.alerts",
			"bark.missing",
		]);
	});
});
