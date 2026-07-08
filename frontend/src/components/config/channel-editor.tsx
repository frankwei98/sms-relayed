import { useState } from "react";
import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import type { AppConfig } from "#/lib/config-model";

const CHANNEL_FIELDS: Record<
	string,
	{ key: string; label: string; defaultValue: string }[]
> = {
	bark: [
		{ key: "server_url", label: "Server URL", defaultValue: "" },
		{ key: "key", label: "Key", defaultValue: "" },
	],
	telegram: [
		{ key: "bot_token", label: "Bot Token", defaultValue: "" },
		{ key: "chat_id", label: "Chat ID", defaultValue: "" },
		{
			key: "api_base",
			label: "API Base",
			defaultValue: "https://api.telegram.org",
		},
	],
	pushplus: [{ key: "token", label: "Token", defaultValue: "" }],
	wecom: [
		{ key: "corp_id", label: "Corp ID", defaultValue: "" },
		{ key: "agent_id", label: "Agent ID", defaultValue: "" },
		{ key: "secret", label: "Secret", defaultValue: "" },
		{ key: "to_user", label: "To User", defaultValue: "@all" },
	],
	dingtalk: [
		{ key: "access_token", label: "Access Token", defaultValue: "" },
		{ key: "secret", label: "Secret", defaultValue: "" },
	],
	shell: [{ key: "path", label: "Path", defaultValue: "" }],
};

type Props = {
	config: AppConfig;
	onUpdate: (config: AppConfig) => void;
};

export function ChannelEditor({ config, onUpdate }: Props) {
	const [newName, setNewName] = useState<Record<string, string>>({});

	function addProfile(channel: string) {
		const name = newName[channel]?.trim();
		if (!name) return;
		const fields = CHANNEL_FIELDS[channel];
		const profile: Record<string, string> = {};
		for (const f of fields) {
			profile[f.key] = f.defaultValue;
		}
		const updated = { ...config };
		updated.channels = {
			...updated.channels,
			[channel]: {
				...updated.channels[channel as keyof typeof updated.channels],
				[name]: profile,
			},
		};
		onUpdate(updated);
		setNewName((prev) => ({ ...prev, [channel]: "" }));
	}

	function removeProfile(channel: string, name: string) {
		const updated = { ...config };
		const profiles = {
			...updated.channels[channel as keyof typeof updated.channels],
		};
		delete profiles[name];
		updated.channels = { ...updated.channels, [channel]: profiles };
		updated.forward = {
			...updated.forward,
			enabled: updated.forward.enabled.filter(
				(ref) => ref !== `${channel}.${name}`,
			),
		};
		onUpdate(updated);
	}

	function updateProfileField(
		channel: string,
		profileName: string,
		field: string,
		value: string,
	) {
		const updated = { ...config };
		const profiles = {
			...updated.channels[channel as keyof typeof updated.channels],
		};
		profiles[profileName] = {
			...profiles[profileName],
			[field]: value,
		};
		updated.channels = { ...updated.channels, [channel]: profiles };
		onUpdate(updated);
	}

	return (
		<div className="space-y-4">
			{(
				["bark", "telegram", "pushplus", "wecom", "dingtalk", "shell"] as const
			).map((channel) => {
				const profiles = config.channels[channel] ?? {};
				const names = Object.keys(profiles);
				return (
					<div key={channel} className="rounded border p-3">
						<h4 className="text-sm font-medium capitalize mb-2">{channel}</h4>
						{names.length === 0 && (
							<p className="text-xs text-muted-foreground mb-2">No profiles</p>
						)}
						{names.map((name) => (
							<div key={name} className="mb-2 rounded bg-muted/30 p-2 text-sm">
								<div className="flex items-center justify-between mb-1">
									<span className="font-medium">{name}</span>
									<Button
										variant="destructive"
										size="sm"
										onClick={() => removeProfile(channel, name)}
									>
										Remove
									</Button>
								</div>
								{CHANNEL_FIELDS[channel].map((field) => (
									<div key={field.key} className="flex items-center gap-2 mt-1">
										<span className="w-24 text-xs text-muted-foreground">
											{field.label}
										</span>
										<Input
											value={
												(profiles[name] as Record<string, string>)[field.key] ??
												""
											}
											onChange={(e) =>
												updateProfileField(
													channel,
													name,
													field.key,
													e.target.value,
												)
											}
											className="h-7 flex-1 text-xs"
											type={
												field.key.includes("token") ||
												field.key.includes("secret") ||
												field.key.includes("key") ||
												field.key === "password"
													? "password"
													: "text"
											}
										/>
									</div>
								))}
							</div>
						))}
						<div className="flex items-center gap-2 mt-2">
							<Input
								placeholder="New profile name"
								value={newName[channel] ?? ""}
								onChange={(e) =>
									setNewName((prev) => ({
										...prev,
										[channel]: e.target.value,
									}))
								}
								className="h-7 flex-1 text-xs"
							/>
							<Button
								variant="outline"
								size="sm"
								onClick={() => addProfile(channel)}
							>
								Add
							</Button>
						</div>
					</div>
				);
			})}
		</div>
	);
}
