import type { Event } from "@sentry/react";
import * as Sentry from "@sentry/react";

const DEFAULT_DSN =
	"https://08650f8b9a69fd934ca178efcee8e62f@o496942.ingest.us.sentry.io/4511733183610880";
const REPORT_INTERVAL_MS = 5 * 60 * 1000;
const lastReports = new Map<string, number>();

export function scrubEvent(event: Event): Event {
	const scrubbed: Event = {
		event_id: event.event_id,
		timestamp: event.timestamp,
		level: event.level,
		platform: event.platform,
		release: event.release,
		environment: event.environment,
		fingerprint: event.fingerprint,
		message: event.message === undefined ? undefined : "[redacted]",
		request: undefined,
		user: undefined,
		breadcrumbs: [],
		extra: {},
		server_name: undefined,
		transaction: undefined,
		contexts: {},
		threads: undefined,
		stacktrace: scrubStacktrace(event.stacktrace),
		tags: event.tags?.status ? { status: event.tags.status } : {},
		exception: event.exception
			? {
					...event.exception,
					values: event.exception.values?.map((exception) => ({
						...exception,
						value: exception.value === undefined ? undefined : "[redacted]",
						stacktrace: scrubStacktrace(exception.stacktrace),
						raw_stacktrace: undefined,
					})),
				}
			: undefined,
	};
	return scrubbed;
}

function scrubStacktrace(stacktrace: Event["stacktrace"]): Event["stacktrace"] {
	if (!stacktrace) return undefined;
	return {
		...stacktrace,
		frames: stacktrace.frames?.map((frame) => ({
			...frame,
			filename: safeBasename(frame.filename ?? frame.abs_path),
			abs_path: undefined,
			pre_context: undefined,
			context_line: undefined,
			post_context: undefined,
			vars: undefined,
		})),
	};
}

function safeBasename(value: string | undefined): string | undefined {
	if (!value) return undefined;
	try {
		const url = new URL(value);
		return url.pathname.split("/").pop() || undefined;
	} catch {
		return value.split(/[\\/]/).pop()?.split(/[?#]/)[0] || undefined;
	}
}

export function initMonitoring(): void {
	Sentry.init({
		dsn: import.meta.env.VITE_SENTRY_DSN || DEFAULT_DSN,
		enabled:
			import.meta.env.PROD && import.meta.env.VITE_SENTRY_ENABLED !== "false",
		// React reports crashes explicitly below; disabling browser defaults also
		// prevents session, navigation, request, and breadcrumb telemetry.
		defaultIntegrations: false,
		environment: import.meta.env.MODE,
		release: import.meta.env.VITE_SENTRY_RELEASE,
		sendDefaultPii: false,
		maxBreadcrumbs: 0,
		beforeBreadcrumb: () => null,
		beforeSend: scrubEvent,
		tracesSampleRate: 0,
	});
}

export function captureFailure(
	code: string,
	tags: { status?: string } = {},
): void {
	const key = `${code}:${tags.status ?? ""}`;
	const now = Date.now();
	const lastReport = lastReports.get(key);
	if (
		lastReport !== undefined &&
		now >= lastReport &&
		now - lastReport < REPORT_INTERVAL_MS
	) {
		return;
	}
	lastReports.set(key, now);

	Sentry.captureEvent({
		level: "error",
		fingerprint: [code],
		exception: { values: [{ type: code }] },
		tags,
	});
}

export { Sentry };
