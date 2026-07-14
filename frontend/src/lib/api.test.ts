import { beforeEach, describe, expect, it, vi } from "vitest";
import { apiFetch } from "./api";

const mocks = vi.hoisted(() => ({ captureFailure: vi.fn() }));

vi.mock("./monitoring", () => ({ captureFailure: mocks.captureFailure }));

describe("apiFetch monitoring", () => {
	beforeEach(() => {
		mocks.captureFailure.mockReset();
	});

	it("reports server errors without sending the request URL", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn().mockResolvedValue(
				new Response(JSON.stringify({ error: { message: "private detail" } }), {
					status: 500,
					headers: { "Content-Type": "application/json" },
				}),
			),
		);

		await expect(
			apiFetch("/api/conversations/+15550000000/read"),
		).rejects.toThrow("private detail");
		expect(mocks.captureFailure).toHaveBeenCalledWith("api.request_failed", {
			status: "500",
		});
	});

	it("does not report expected client errors", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn().mockResolvedValue(
				new Response(JSON.stringify({ error: { message: "unauthorized" } }), {
					status: 401,
					headers: { "Content-Type": "application/json" },
				}),
			),
		);

		await expect(apiFetch("/api/auth/me")).rejects.toThrow("unauthorized");
		expect(mocks.captureFailure).not.toHaveBeenCalled();
	});

	it("reports network failures without sending the request URL", async () => {
		vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("offline")));

		await expect(apiFetch("/api/messages?phone=+15550000000")).rejects.toThrow(
			"offline",
		);
		expect(mocks.captureFailure).toHaveBeenCalledWith("api.request_failed", {
			status: "network_error",
		});
	});
});
