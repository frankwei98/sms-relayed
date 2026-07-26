import { useState } from "react";
import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import { Switch } from "#/components/ui/switch";
import type { AppConfig } from "#/lib/config-model";

const CHANNEL_FIELDS: Record<
	string,
	{ key: string; label: string; defaultValue: string; sensitive?: boolean }[]
> = {
	bark: [
		{ key: "server_url", label: "Server URL", defaultValue: "" },
		{ key: "key", label: "Key", defaultValue: "", sensitive: true },
	],
	telegram: [
		{
			key: "bot_token",
			label: "Bot Token",
			defaultValue: "",
			sensitive: true,
		},
		{ key: "chat_id", label: "Chat ID", defaultValue: "" },
		{
			key: "api_base",
			label: "API Base",
			defaultValue: "https://api.telegram.org",
		},
	],
	wecom: [
		{ key: "corp_id", label: "Corp ID", defaultValue: "" },
		{ key: "agent_id", label: "Agent ID", defaultValue: "" },
		{ key: "secret", label: "Secret", defaultValue: "", sensitive: true },
		{ key: "to_user", label: "To User", defaultValue: "@all" },
	],
	dingtalk: [
		{
			key: "access_token",
			label: "Access Token",
			defaultValue: "",
			sensitive: true,
		},
		{ key: "secret", label: "Secret", defaultValue: "", sensitive: true },
	],
	lark: [
		{
			key: "webhook_url",
			label: "Webhook URL",
			defaultValue: "",
			sensitive: true,
		},
		{
			key: "secret",
			label: "Signing Secret",
			defaultValue: "",
			sensitive: true,
		},
	],
	shell: [{ key: "path", label: "Path", defaultValue: "" }],
};

const CHANNELS = [
	"bark",
	"telegram",
	"wecom",
	"dingtalk",
	"lark",
	"shell",
] as const;

type Props = {
	config: AppConfig;
	onUpdate: (config: AppConfig) => void;
};

function getProfileRef(channel: string, name: string) {
	return `${channel}.${name}`;
}

function getProfileRefs(config: AppConfig) {
	return new Set(
		CHANNELS.flatMap((channel) =>
			Object.keys(config.channels[channel] ?? {}).map((name) =>
				getProfileRef(channel, name),
			),
		),
	);
}

export function normalizeForwardEnabled(config: AppConfig) {
	const profileRefs = getProfileRefs(config);
	return {
		...config,
		forward: {
			...config.forward,
			enabled: config.forward.enabled.filter((ref) => profileRefs.has(ref)),
		},
	};
}

export function ChannelEditor({ config, onUpdate }: Props) {
	const [newName, setNewName] = useState<Record<string, string>>({});
	const enabledRefs = new Set(config.forward.enabled);
	const profileRefs = getProfileRefs(config);
	const profileCount = profileRefs.size;
	const enabledCount = [...profileRefs].filter((ref) =>
		enabledRefs.has(ref),
	).length;

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
				(ref) => ref !== getProfileRef(channel, name),
			),
		};
		onUpdate(updated);
	}

	function setProfileEnabled(channel: string, name: string, enabled: boolean) {
		const profileRef = getProfileRef(channel, name);
		const currentEnabled = config.forward.enabled.filter(
			(ref) => profileRefs.has(ref) && ref !== profileRef,
		);
		const nextEnabled = enabled
			? [...currentEnabled, profileRef]
			: currentEnabled;
		onUpdate({
			...config,
			forward: {
				...config.forward,
				enabled: nextEnabled,
			},
		});
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
			<div className="flex items-center justify-between gap-3 rounded-xl border bg-muted/20 px-4 py-3">
				<div>
					<p className="text-sm font-medium">Delivery routes</p>
					<p className="text-xs text-muted-foreground">
						Enable the profiles that should receive forwarded messages.
					</p>
				</div>
				<p className="font-mono text-xs text-muted-foreground">
					<span className="font-semibold text-foreground">{enabledCount}</span>{" "}
					/ {profileCount} active
				</p>
			</div>
			{CHANNELS.map((channel) => {
				const profiles = config.channels[channel] ?? {};
				const names = Object.keys(profiles);
				return (
					<div key={channel} className="rounded-xl border bg-card/30 p-3">
						<h4 className="mb-2 text-sm font-medium capitalize">{channel}</h4>
						{names.length === 0 && (
							<p className="mb-2 text-xs text-muted-foreground">No profiles</p>
						)}
						{names.map((name) => {
							const profileRef = getProfileRef(channel, name);
							const enabled = enabledRefs.has(profileRef);
							return (
								<div
									key={name}
									className={`mb-2 rounded-xl border p-3 text-sm transition-colors ${
										enabled
											? "border-primary/35 bg-primary/5"
											: "border-transparent bg-muted/30"
									}`}
								>
									<div className="mb-2 flex flex-col items-stretch justify-between gap-3 sm:flex-row sm:items-center">
										<div className="min-w-0">
											<p className="font-medium">{name}</p>
											<p className="truncate font-mono text-[11px] text-muted-foreground">
												{profileRef}
											</p>
										</div>
										<div className="flex shrink-0 items-center justify-between gap-3 sm:justify-end">
											<div className="flex items-center gap-2 text-xs">
												<span
													className={
														enabled
															? "font-medium text-foreground"
															: "text-muted-foreground"
													}
												>
													{enabled ? "Enabled" : "Disabled"}
												</span>
												<Switch
													size="sm"
													checked={enabled}
													aria-label={`Enable forwarding for ${profileRef}`}
													onCheckedChange={(checked: boolean) =>
														setProfileEnabled(channel, name, checked)
													}
												/>
											</div>
											<Button
												variant="destructive"
												size="sm"
												aria-label={`Remove ${profileRef}`}
												onClick={() => removeProfile(channel, name)}
											>
												Remove
											</Button>
										</div>
									</div>
									{CHANNEL_FIELDS[channel].map((field) => (
										<div
											key={field.key}
											className="mt-1 flex items-center gap-2"
										>
											<span className="w-24 text-xs text-muted-foreground">
												{field.label}
											</span>
											<Input
												value={
													(profiles[name] as Record<string, string>)[
														field.key
													] ?? ""
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
												type={field.sensitive ? "password" : "text"}
											/>
										</div>
									))}
								</div>
							);
						})}
						<div className="mt-2 flex items-center gap-2">
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
