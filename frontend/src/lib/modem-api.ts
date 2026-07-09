import { apiFetch } from "#/lib/api";

export type HealthLevel = "ok" | "degraded" | "error" | "unknown";

export type ModemStatus = {
	checked_at: string;
	tool: {
		available: boolean;
		version_raw: string | null;
		supports_json: boolean;
	};
	configured_modem_path: string;
	resolved: {
		present: boolean;
		id: string | null;
		path: string | null;
	};
	health: {
		status: HealthLevel;
		reasons: string[];
	};
	modem: {
		enabled: boolean | null;
		state: string | null;
		sim_state: string | null;
		operator_name: string | null;
		signal_quality: number | null;
		access_technologies: string[];
	};
	messaging: {
		available: boolean;
		supported_storages: string[];
		default_storage: string | null;
	};
	diagnostics: {
		last_error: string | null;
		path_drift_candidate: string | null;
	};
};

export type ModemAction = "enable" | "disable" | "reset";

export type ActionResponse = {
	accepted: boolean;
	action: ModemAction;
};

export function fetchModemStatus() {
	return apiFetch<ModemStatus>("/api/modem/status");
}

export function runModemAction(action: ModemAction) {
	return apiFetch<ActionResponse>(`/api/modem/${action}`, {
		method: "POST",
		body: JSON.stringify(action === "reset" ? { confirm: true } : {}),
	});
}
