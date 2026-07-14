import { describe, expect, it, vi } from "vitest";
import { captureFailure, scrubEvent } from "./monitoring";

const mocks = vi.hoisted(() => ({ captureEvent: vi.fn() }));

vi.mock("@sentry/react", () => ({ captureEvent: mocks.captureEvent }));

describe("Sentry event privacy", () => {
	it("removes user-controlled payloads while keeping exception identity", () => {
		const event = scrubEvent({
			message: "SMS body 123456",
			logentry: { message: "phone=+15550000000" },
			logger: "router-bedroom",
			request: { url: "/api/conversations/+15550000000/read" },
			user: { id: "+15550000000" },
			breadcrumbs: [{ message: "token=secret" }],
			extra: { config: { token: "secret" } },
			exception: {
				values: [
					{
						type: "TypeError",
						value: "failed while rendering SMS body",
						stacktrace: {
							frames: [
								{
									filename: "https://router.local/assets/main.tsx?build=secret",
									abs_path: "https://router.local/assets/main.tsx",
									context_line: "const token = 'secret'",
								},
								{ filename: "https://router.local" },
							],
						},
					},
				],
			},
		});

		expect(event.message).toBe("[redacted]");
		expect(event.logentry).toBeUndefined();
		expect(event.logger).toBeUndefined();
		expect(event.request).toBeUndefined();
		expect(event.user).toBeUndefined();
		expect(event.breadcrumbs).toEqual([]);
		expect(event.extra).toEqual({});
		expect(event.exception?.values?.[0]?.type).toBe("TypeError");
		expect(event.exception?.values?.[0]?.value).toBe("[redacted]");
		expect(event.exception?.values?.[0]?.stacktrace?.frames).toHaveLength(2);
		expect(event.exception?.values?.[0]?.stacktrace?.frames?.[0]).toMatchObject(
			{
				filename: "main.tsx",
				abs_path: undefined,
				context_line: undefined,
			},
		);
		expect(
			event.exception?.values?.[0]?.stacktrace?.frames?.[1]?.filename,
		).toBeUndefined();
	});

	it("throttles repeated operational failures by code and status", () => {
		vi.useFakeTimers();
		vi.setSystemTime(new Date("2026-07-14T00:00:00Z"));

		captureFailure("api.request_failed", { status: "500" });
		captureFailure("api.request_failed", { status: "500" });
		captureFailure("api.request_failed", { status: "503" });
		vi.setSystemTime(new Date("2026-07-13T23:59:00Z"));
		captureFailure("api.request_failed", { status: "500" });

		expect(mocks.captureEvent).toHaveBeenCalledTimes(3);
		vi.useRealTimers();
	});
});
