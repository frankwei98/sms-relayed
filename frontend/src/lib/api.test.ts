// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError, apiDownload, apiFetch, apiRequest } from "./api";

const mocks = vi.hoisted(() => ({ captureFailure: vi.fn() }));

vi.mock("./monitoring", () => ({ captureFailure: mocks.captureFailure }));

describe("apiFetch monitoring", () => {
	beforeEach(() => {
		mocks.captureFailure.mockReset();
	});

	it("accepts a successful response with an empty body", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn().mockResolvedValue(new Response(null, { status: 202 })),
		);

		await expect(
			apiFetch("/api/service/restart", { method: "POST" }),
		).resolves.toBeUndefined();
		expect(mocks.captureFailure).not.toHaveBeenCalled();
	});

	it("accepts a successful response with a whitespace-only body", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn().mockResolvedValue(new Response(" \n\t", { status: 202 })),
		);

		await expect(
			apiFetch("/api/service/restart", { method: "POST" }),
		).resolves.toBeUndefined();
		expect(mocks.captureFailure).not.toHaveBeenCalled();
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
		const unauthorized = vi.fn();
		window.addEventListener("sms-relayed:unauthorized", unauthorized);
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
		expect(unauthorized).toHaveBeenCalledOnce();
		window.removeEventListener("sms-relayed:unauthorized", unauthorized);
	});

	it("broadcasts unauthorized responses from file downloads", async () => {
		const unauthorized = vi.fn();
		window.addEventListener("sms-relayed:unauthorized", unauthorized);
		vi.stubGlobal(
			"fetch",
			vi.fn().mockResolvedValue(new Response(null, { status: 401 })),
		);

		await expect(apiDownload("/api/messages/export")).rejects.toThrow(
			"Request failed: 401",
		);
		expect(unauthorized).toHaveBeenCalledOnce();
		window.removeEventListener("sms-relayed:unauthorized", unauthorized);
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

	it("preserves status and backend error code", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn().mockResolvedValue(
				new Response(
					JSON.stringify({
						error: { code: "config_changed", message: "reload first" },
					}),
					{ status: 412, headers: { "Content-Type": "application/json" } },
				),
			),
		);

		const error = await apiFetch("/api/config").catch((caught) => caught);
		expect(error).toBeInstanceOf(ApiRequestError);
		expect((error as ApiRequestError).status).toBe(412);
		expect((error as ApiRequestError).code).toBe("config_changed");
	});

	it("uses status-aware fallbacks when an error response omits the error body", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn().mockResolvedValue(
				new Response(JSON.stringify({}), {
					status: 429,
					headers: { "Content-Type": "application/json" },
				}),
			),
		);

		const error = await apiFetch("/api/config").catch((caught) => caught);
		expect(error).toBeInstanceOf(ApiRequestError);
		expect((error as ApiRequestError).status).toBe(429);
		expect((error as ApiRequestError).code).toBe("request_failed");
		expect((error as ApiRequestError).message).toBe("Request failed: 429");
	});

	it("returns response metadata and merges caller headers", async () => {
		const fetchMock = vi.fn().mockResolvedValue(
			new Response(JSON.stringify({ value: 1 }), {
				status: 200,
				headers: { ETag: '"revision"' },
			}),
		);
		vi.stubGlobal("fetch", fetchMock);

		const result = await apiRequest<{ value: number }>("/api/config", {
			headers: new Headers({
				"Content-Type": "application/merge-patch+json",
				"If-Match": "base",
			}),
		});

		expect(result.data).toEqual({ value: 1 });
		expect(result.response.headers.get("etag")).toBe('"revision"');
		const requestHeaders = new Headers(fetchMock.mock.calls[0][1].headers);
		expect(requestHeaders.get("content-type")).toBe(
			"application/merge-patch+json",
		);
		expect(requestHeaders.get("if-match")).toBe("base");
	});
});
