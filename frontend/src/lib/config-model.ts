export type AppConfig = {
	app: {
		device_name: string;
		modem_path: string;
	};
	sms: {
		ignore_storage: string[];
		code_keywords: string[];
	};
	forward: {
		enabled: string[];
	};
	channels: {
		bark: Record<string, { server_url: string; key: string }>;
		telegram: Record<
			string,
			{ bot_token: string; chat_id: string; api_base: string }
		>;
		pushplus: Record<string, { token: string }>;
		wecom: Record<
			string,
			{ corp_id: string; agent_id: string; secret: string; to_user: string }
		>;
		dingtalk: Record<string, { access_token: string; secret: string }>;
		shell: Record<string, { path: string }>;
	};
	api: {
		enabled: boolean;
		bind: string;
		port: number;
		enable_ipv6: boolean;
		password: string;
		database_path: string;
	};
};

export type StatusResponse = {
	version: string;
	uptime_seconds: number;
	api_bind: string;
	api_port: number;
	database_path: string;
};
