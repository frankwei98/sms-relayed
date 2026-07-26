import { useState } from "react";
import { Button } from "#/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "#/components/ui/dialog";
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

export function ChannelEditor({ config, onUpdate }: Props) {
	const [newName, setNewName] = useState<Record<string, string>>({});
	const [pendingRemoval, setPendingRemoval] = useState<{
		channel: string;
		name: string;
	} | null>(null);
	const enabledRefs = new Set(config.forward.enabled);
	const profileRefs = getProfileRefs(config);
	const profileCount = profileRefs.size;
	const enabledCount = [...profileRefs].filter((ref) =>
		enabledRefs.has(ref),
	).length;
	const missingProfileRefs = [
		...new Set(config.forward.enabled.filter((ref) => !profileRefs.has(ref))),
	];

	function addProfile(channel: string) {
		const name = newName[channel]?.trim();
		const profiles = config.channels[
			channel as keyof AppConfig["channels"]
		] as Record<string, unknown>;
		if (!name || name in profiles) return;
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
			(ref) => ref !== profileRef,
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

	function removeMissingProfileRef(profileRef: string) {
		onUpdate({
			...config,
			forward: {
				...config.forward,
				enabled: config.forward.enabled.filter((ref) => ref !== profileRef),
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
			{missingProfileRefs.length > 0 ? (
				<section
					aria-labelledby="missing-forwarding-profiles"
					className="rounded-xl border border-amber-500/35 bg-amber-500/10 p-4"
				>
					<div>
						<h4
							id="missing-forwarding-profiles"
							className="text-sm font-medium text-amber-950 dark:text-amber-100"
						>
							Missing forwarding profiles
						</h4>
						<p className="mt-1 text-xs text-amber-900/80 dark:text-amber-100/75">
							These enabled references do not match a configured profile. Remove
							them to make this configuration valid.
						</p>
					</div>
					<ul className="mt-3 space-y-2">
						{missingProfileRefs.map((profileRef) => (
							<li
								key={profileRef}
								className="flex flex-col gap-2 rounded-lg border border-amber-500/25 bg-background/70 px-3 py-2 sm:flex-row sm:items-center sm:justify-between"
							>
								<code className="min-w-0 break-all text-xs">{profileRef}</code>
								<Button
									variant="outline"
									size="sm"
									className="shrink-0 self-start border-amber-500/35 sm:self-auto"
									aria-label={`Remove missing forwarding reference ${profileRef}`}
									onClick={() => removeMissingProfileRef(profileRef)}
								>
									Remove reference
								</Button>
							</li>
						))}
					</ul>
				</section>
			) : null}
			{CHANNELS.map((channel) => {
				const profiles = config.channels[channel] ?? {};
				const names = Object.keys(profiles);
				const candidateName = newName[channel]?.trim() ?? "";
				const duplicateName =
					candidateName.length > 0 && candidateName in profiles;
				const newProfileId = `new-${channel}-profile`;
				const duplicateNameId = `${newProfileId}-duplicate`;
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
												onClick={() => setPendingRemoval({ channel, name })}
											>
												Remove
											</Button>
										</div>
									</div>
									{CHANNEL_FIELDS[channel].map((field) => {
										const fieldId = `channel-${encodeURIComponent(profileRef)}-${field.key}`;
										return (
											<div
												key={field.key}
												className="mt-2 grid gap-1.5 sm:grid-cols-[7rem_minmax(0,1fr)] sm:items-center"
											>
												<label
													htmlFor={fieldId}
													className="text-xs text-muted-foreground"
												>
													{field.label}
												</label>
												<Input
													id={fieldId}
													value={
														(profiles[name] as Record<string, string>)[
															field.key
														] ?? ""
													}
													onChange={(event) =>
														updateProfileField(
															channel,
															name,
															field.key,
															event.target.value,
														)
													}
													className="h-8 text-xs"
													type={field.sensitive ? "password" : "text"}
													autoComplete={field.sensitive ? "off" : undefined}
												/>
											</div>
										);
									})}
								</div>
							);
						})}
						<div className="mt-3 border-t pt-3">
							<label htmlFor={newProfileId} className="text-xs font-medium">
								Add {channel} profile
							</label>
							<div className="mt-1.5 flex items-center gap-2">
								<Input
									id={newProfileId}
									placeholder="Profile name"
									value={newName[channel] ?? ""}
									onChange={(e) =>
										setNewName((prev) => ({
											...prev,
											[channel]: e.target.value,
										}))
									}
									className="h-8 flex-1 text-xs"
									aria-invalid={duplicateName}
									aria-describedby={duplicateName ? duplicateNameId : undefined}
								/>
								<Button
									variant="outline"
									size="sm"
									disabled={!candidateName || duplicateName}
									onClick={() => addProfile(channel)}
								>
									Add
								</Button>
							</div>
							{duplicateName ? (
								<p
									id={duplicateNameId}
									className="mt-1 text-xs text-destructive"
								>
									That profile name already exists.
								</p>
							) : null}
						</div>
					</div>
				);
			})}
			<Dialog
				open={pendingRemoval !== null}
				onOpenChange={(open) => {
					if (!open) setPendingRemoval(null);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Remove forwarding profile?</DialogTitle>
						<DialogDescription>
							This removes the profile credentials and its enabled reference
							from the current draft. The change is not written until you save.
						</DialogDescription>
					</DialogHeader>
					{pendingRemoval ? (
						<p className="rounded-md bg-muted px-3 py-2 font-mono text-xs">
							{getProfileRef(pendingRemoval.channel, pendingRemoval.name)}
						</p>
					) : null}
					<DialogFooter>
						<Button variant="outline" onClick={() => setPendingRemoval(null)}>
							Cancel
						</Button>
						<Button
							variant="destructive"
							onClick={() => {
								if (!pendingRemoval) return;
								removeProfile(pendingRemoval.channel, pendingRemoval.name);
								setPendingRemoval(null);
							}}
						>
							Remove profile
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}
