// @vitest-environment jsdom

import {
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { PhoneNumberCopy } from "#/components/phone-number-copy";

afterEach(() => {
	cleanup();
	vi.restoreAllMocks();
	Object.defineProperty(navigator, "clipboard", {
		configurable: true,
		value: undefined,
	});
	Object.defineProperty(document, "execCommand", {
		configurable: true,
		value: undefined,
	});
});

describe("PhoneNumberCopy", () => {
	test("copies the complete number and shows success feedback", async () => {
		const writeText = vi.fn().mockResolvedValue(undefined);
		Object.defineProperty(navigator, "clipboard", {
			configurable: true,
			value: { writeText },
		});

		render(<PhoneNumberCopy phoneNumber="+6581234567" />);
		fireEvent.click(screen.getByRole("button", { name: "Copy phone number" }));

		await waitFor(() => {
			expect(writeText).toHaveBeenCalledWith("+6581234567");
		});
		expect(await screen.findByText("Copied")).toBeTruthy();
	});

	test("falls back to document copy when the Clipboard API is unavailable", async () => {
		const execCommand = vi.fn().mockReturnValue(true);
		Object.defineProperty(document, "execCommand", {
			configurable: true,
			value: execCommand,
		});

		render(<PhoneNumberCopy phoneNumber="+6581234567" />);
		fireEvent.click(screen.getByRole("button", { name: "Copy phone number" }));

		await waitFor(() => {
			expect(execCommand).toHaveBeenCalledWith("copy");
		});
		expect(await screen.findByText("Copied")).toBeTruthy();
	});

	test("falls back when the Clipboard API rejects the request", async () => {
		const writeText = vi.fn().mockRejectedValue(new Error("not allowed"));
		const execCommand = vi.fn().mockReturnValue(true);
		Object.defineProperty(navigator, "clipboard", {
			configurable: true,
			value: { writeText },
		});
		Object.defineProperty(document, "execCommand", {
			configurable: true,
			value: execCommand,
		});

		render(<PhoneNumberCopy phoneNumber="+6581234567" />);
		fireEvent.click(screen.getByRole("button", { name: "Copy phone number" }));

		await waitFor(() => {
			expect(execCommand).toHaveBeenCalledWith("copy");
		});
		expect(await screen.findByText("Copied")).toBeTruthy();
	});

	test("shows failure feedback when neither copy method works", async () => {
		const execCommand = vi.fn().mockReturnValue(false);
		Object.defineProperty(document, "execCommand", {
			configurable: true,
			value: execCommand,
		});

		render(<PhoneNumberCopy phoneNumber="+6581234567" />);
		fireEvent.click(screen.getByRole("button", { name: "Copy phone number" }));

		expect(await screen.findByText("Copy failed")).toBeTruthy();
	});
});
