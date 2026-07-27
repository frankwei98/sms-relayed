import { apiFetch } from "#/lib/api";

export type HealthLevel = "ok" | "degraded" | "error" | "unknown";

export type SmsOverIms = {
	status:
		| "available"
		| "registering"
		| "limited"
		| "not_registered"
		| "unavailable"
		| "unknown";
	support: "supported" | "unsupported" | "unknown";
	configured: "enabled" | "disabled" | "unknown";
	registration:
		| "registered"
		| "registering"
		| "limited"
		| "not_registered"
		| "unknown";
	sms_service: "available" | "limited" | "unavailable" | "unknown";
	technology: "wwan" | "wlan" | "interworking_wlan" | "unknown";
	probe: {
		tool: string;
		available: boolean;
		version_raw: string | null;
		transport: "direct_qmi" | "unknown";
		device: string | null;
		capabilities: {
			ims_settings: boolean;
			imsa_registration: boolean;
			imsa_services: boolean;
		};
	};
	evidence: string[];
	reasons: string[];
	warnings: string[];
};

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
		own_number: string | null;
		operator_name: string | null;
		signal_quality: number | null;
		access_technologies: string[];
	};
	messaging: {
		available: boolean;
		supported_storages: string[];
		default_storage: string | null;
	};
	sms_over_ims: SmsOverIms;
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
