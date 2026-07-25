// @vitest-environment jsdom

import {
	act,
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
	vi.useRealTimers();
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
		expect(screen.getByRole("status").textContent).toBe("Phone number copied");
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
		expect(screen.getByRole("status").textContent).toBe(
			"Phone number copy failed",
		);
	});

	test("ignores an older copy request that completes after the latest request", async () => {
		vi.useFakeTimers();
		let resolveFirst: (() => void) | undefined;
		let rejectFirst: ((reason: Error) => void) | undefined;
		let resolveSecond: (() => void) | undefined;
		const first = new Promise<void>((resolve, reject) => {
			resolveFirst = resolve;
			rejectFirst = reject;
		});
		const second = new Promise<void>((resolve) => {
			resolveSecond = resolve;
		});
		const writeText = vi
			.fn()
			.mockReturnValueOnce(first)
			.mockReturnValueOnce(second);
		Object.defineProperty(navigator, "clipboard", {
			configurable: true,
			value: { writeText },
		});
		Object.defineProperty(document, "execCommand", {
			configurable: true,
			value: vi.fn().mockReturnValue(false),
		});

		render(<PhoneNumberCopy phoneNumber="+6581234567" />);
		const button = screen.getByRole("button", { name: "Copy phone number" });
		fireEvent.click(button);
		fireEvent.click(button);

		await act(async () => {
			resolveSecond?.();
			await second;
		});
		expect(screen.getByText("Copied")).toBeTruthy();

		await act(async () => {
			await vi.advanceTimersByTimeAsync(1000);
			rejectFirst?.(new Error("older request failed"));
			await first.catch(() => {});
		});
		expect(screen.queryByText("Copy failed")).toBeNull();
		expect(screen.getByText("Copied")).toBeTruthy();

		await act(async () => {
			await vi.advanceTimersByTimeAsync(1000);
		});
		expect(screen.getByText("Copy")).toBeTruthy();

		resolveFirst?.();
	});
});
