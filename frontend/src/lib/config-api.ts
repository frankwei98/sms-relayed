import { apiFetch, apiRequest } from "#/lib/api";
import type { AppConfig } from "#/lib/config-model";

export type ConfigDocument = {
	config: AppConfig;
	revision: string;
	restartRequired: boolean;
};

export type ConfigWarning =
	| "password_change"
	| "api_disable"
	| "api_endpoint_change"
	| "database_path_change";

export type ConfigPreviewResponse = {
	base_revision: string;
	candidate_revision: string;
	has_changes: boolean;
	diff: string;
	check: {
		passed: boolean;
		message: string | null;
	};
	requires_restart: boolean;
	password_change_pending: boolean;
	warnings: ConfigWarning[];
};

export type ConfigSaveResponse = {
	revision: string;
	requires_restart: boolean;
	restart_scheduled: boolean;
	session_invalidated: boolean;
};

function parseRevision(value: string | null): string {
	if (!value) {
		throw new Error("Config response did not include a revision.");
	}
	return value.replace(/^W\//, "").replace(/^"|"$/g, "");
}

export async function loadConfigDocument(): Promise<ConfigDocument> {
	const { data, response } = await apiRequest<AppConfig>("/api/config");
	return {
		config: data,
		revision: parseRevision(response.headers.get("etag")),
		restartRequired:
			response.headers.get("x-config-restart-required") === "true",
	};
}

export async function checkConfig(config: AppConfig): Promise<void> {
	await apiFetch("/api/config/check", {
		method: "POST",
		body: JSON.stringify(config),
	});
}

export async function previewConfig(
	config: AppConfig,
	baseRevision: string,
): Promise<ConfigPreviewResponse> {
	return apiFetch<ConfigPreviewResponse>("/api/config/preview", {
		method: "POST",
		headers: { "If-Match": baseRevision },
		body: JSON.stringify(config),
	});
}

export async function saveConfig(
	config: AppConfig,
	baseRevision: string,
	candidateRevision: string,
	restartAfterSave: boolean,
): Promise<ConfigSaveResponse> {
	const query = restartAfterSave ? "?restart_after_save=true" : "";
	return apiFetch<ConfigSaveResponse>(`/api/config${query}`, {
		method: "PUT",
		headers: {
			"If-Match": baseRevision,
			"X-Config-Candidate-Revision": candidateRevision,
		},
		body: JSON.stringify(config),
	});
}

export async function scheduleRestart(): Promise<void> {
	await apiFetch("/api/service/restart", { method: "POST" });
}
