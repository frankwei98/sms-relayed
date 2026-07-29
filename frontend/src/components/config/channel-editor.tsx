import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
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
import { Textarea } from "#/components/ui/textarea";
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
	webhook: [],
};

const CHANNELS = [
	"bark",
	"telegram",
	"wecom",
	"dingtalk",
	"lark",
	"webhook",
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
	const { t } = useTranslation();
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
		const profile: Record<string, unknown> =
			channel === "webhook"
				? {
						method: "post",
						url: "",
						content_type: "application/json",
						body: '{\n  "sender": {SENDER_JSON},\n  "message": {MESSAGE_JSON},\n  "datetime": {DATETIME_JSON}\n}',
						headers: {},
					}
				: Object.fromEntries(
						CHANNEL_FIELDS[channel].map((field) => [
							field.key,
							field.defaultValue,
						]),
					);
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

	function updateWebhookHeader(
		profileName: string,
		oldName: string,
		name: string,
		value: string,
	) {
		const profile = config.channels.webhook[profileName];
		const headers = { ...profile.headers };
		delete headers[oldName];
		if (name) headers[name] = value;
		onUpdate({
			...config,
			channels: {
				...config.channels,
				webhook: {
					...config.channels.webhook,
					[profileName]: { ...profile, headers },
				},
			},
		});
	}

	function addWebhookHeader(profileName: string) {
		const profile = config.channels.webhook[profileName];
		let index = 1;
		let name = "X-Custom-Header";
		while (name in profile.headers) {
			index += 1;
			name = `X-Custom-Header-${index}`;
		}
		updateWebhookHeader(profileName, "", name, "");
	}

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between gap-3 rounded-xl border bg-muted/20 px-4 py-3">
				<div>
					<p className="text-sm font-medium">
						{t("config.channel.deliveryRoutes")}
					</p>
					<p className="text-xs text-muted-foreground">
						{t("config.channel.deliveryRoutesDescription")}
					</p>
				</div>
				<p className="font-mono text-xs text-muted-foreground">
					{t("config.channel.profilesActive", {
						enabled: enabledCount,
						total: profileCount,
					})}
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
							{t("config.channel.missingProfiles")}
						</h4>
						<p className="mt-1 text-xs text-amber-900/80 dark:text-amber-100/75">
							{t("config.channel.missingProfilesDescription")}
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
									aria-label={t("config.channel.removeReferenceAria", {
										ref: profileRef,
									})}
									onClick={() => removeMissingProfileRef(profileRef)}
								>
									{t("config.channel.removeReference")}
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
							<p className="mb-2 text-xs text-muted-foreground">
								{t("config.channel.noProfiles")}
							</p>
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
													{enabled
														? t("config.channel.enabled")
														: t("config.channel.disabled")}
												</span>
												<Switch
													size="sm"
													checked={enabled}
													aria-label={t("config.channel.enableAria", {
														ref: profileRef,
													})}
													onCheckedChange={(checked: boolean) =>
														setProfileEnabled(channel, name, checked)
													}
												/>
											</div>
											<Button
												variant="destructive"
												size="sm"
												aria-label={t("config.channel.removeAria", {
													ref: profileRef,
												})}
												onClick={() => setPendingRemoval({ channel, name })}
											>
												{t("config.channel.remove")}
											</Button>
										</div>
									</div>
									{channel === "webhook" ? (
										<WebhookFields
											profileName={name}
											profile={config.channels.webhook[name]}
											onFieldChange={(field, value) =>
												updateProfileField(channel, name, field, value)
											}
											onHeaderChange={(oldName, headerName, value) =>
												updateWebhookHeader(name, oldName, headerName, value)
											}
											onAddHeader={() => addWebhookHeader(name)}
										/>
									) : (
										CHANNEL_FIELDS[channel].map((field) => {
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
										})
									)}
								</div>
							);
						})}
						<div className="mt-3 border-t pt-3">
							<label htmlFor={newProfileId} className="text-xs font-medium">
								{t("config.channel.addProfile", { channel })}
							</label>
							<div className="mt-1.5 flex items-center gap-2">
								<Input
									id={newProfileId}
									placeholder={t("config.channel.profileName")}
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
									{t("config.channel.add")}
								</Button>
							</div>
							{duplicateName ? (
								<p
									id={duplicateNameId}
									className="mt-1 text-xs text-destructive"
								>
									{t("config.channel.duplicateName")}
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
						<DialogTitle>{t("config.channel.removeDialog.title")}</DialogTitle>
						<DialogDescription>
							{t("config.channel.removeDialog.description")}
						</DialogDescription>
					</DialogHeader>
					{pendingRemoval ? (
						<p className="rounded-md bg-muted px-3 py-2 font-mono text-xs">
							{getProfileRef(pendingRemoval.channel, pendingRemoval.name)}
						</p>
					) : null}
					<DialogFooter>
						<Button variant="outline" onClick={() => setPendingRemoval(null)}>
							{t("config.channel.removeDialog.cancel")}
						</Button>
						<Button
							variant="destructive"
							onClick={() => {
								if (!pendingRemoval) return;
								removeProfile(pendingRemoval.channel, pendingRemoval.name);
								setPendingRemoval(null);
							}}
						>
							{t("config.channel.removeDialog.remove")}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}

function WebhookFields({
	profileName,
	profile,
	onFieldChange,
	onHeaderChange,
	onAddHeader,
}: {
	profileName: string;
	profile: AppConfig["channels"]["webhook"][string];
	onFieldChange: (field: string, value: string) => void;
	onHeaderChange: (oldName: string, name: string, value: string) => void;
	onAddHeader: () => void;
}) {
	const { t } = useTranslation();
	const prefix = `channel-webhook.${profileName}`;
	return (
		<div className="space-y-2">
			<div className="grid gap-1.5 sm:grid-cols-[7rem_minmax(0,1fr)] sm:items-center">
				<label
					htmlFor={`${prefix}-method`}
					className="text-xs text-muted-foreground"
				>
					{t("config.channel.webhookMethod")}
				</label>
				<select
					id={`${prefix}-method`}
					value={profile.method}
					onChange={(event) => {
						const method = event.target.value;
						onFieldChange("method", method);
					}}
					className="h-8 rounded-md border bg-background px-2 text-xs"
				>
					<option value="post">POST</option>
					<option value="get">GET</option>
				</select>
			</div>
			{profile.method === "get" ? (
				<p className="rounded-md border border-amber-500/35 bg-amber-500/10 px-3 py-2 text-xs text-amber-950 dark:text-amber-100">
					{t("config.channel.webhookGetWarning")}
				</p>
			) : null}
			<WebhookTextField
				id={`${prefix}-url`}
				label={t("config.channel.webhookUrl")}
				value={profile.url}
				onChange={(value) => onFieldChange("url", value)}
			/>
			{profile.method === "post" ? (
				<>
					<WebhookTextField
						id={`${prefix}-content-type`}
						label={t("config.channel.webhookContentType")}
						value={profile.content_type}
						onChange={(value) => onFieldChange("content_type", value)}
					/>
					<div className="grid gap-1.5 sm:grid-cols-[7rem_minmax(0,1fr)]">
						<label
							htmlFor={`${prefix}-body`}
							className="pt-2 text-xs text-muted-foreground"
						>
							{t("config.channel.webhookBody")}
						</label>
						<Textarea
							id={`${prefix}-body`}
							value={profile.body}
							onChange={(event) => onFieldChange("body", event.target.value)}
							className="min-h-28 font-mono text-xs"
						/>
					</div>
				</>
			) : null}
			<div className="grid gap-1.5 sm:grid-cols-[7rem_minmax(0,1fr)]">
				<span className="pt-2 text-xs text-muted-foreground">
					{t("config.channel.webhookHeaders")}
				</span>
				<div className="space-y-2">
					{Object.entries(profile.headers).map(([name, value]) => (
						<WebhookHeaderRow
							key={name}
							name={name}
							value={value}
							allNames={Object.keys(profile.headers)}
							onChange={onHeaderChange}
						/>
					))}
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={onAddHeader}
					>
						{t("config.channel.webhookAddHeader")}
					</Button>
				</div>
			</div>
			<p className="text-xs text-muted-foreground">
				{t("config.channel.webhookVariables")}
			</p>
		</div>
	);
}

function WebhookHeaderRow({
	name,
	value,
	allNames,
	onChange,
}: {
	name: string;
	value: string;
	allNames: string[];
	onChange: (oldName: string, name: string, value: string) => void;
}) {
	const { t } = useTranslation();
	const errorId = useId();
	const [draftName, setDraftName] = useState(name);
	const missing = draftName.length === 0;
	const duplicate =
		draftName.toLowerCase() !== name.toLowerCase() &&
		allNames.some(
			(existingName) => existingName.toLowerCase() === draftName.toLowerCase(),
		);
	const invalid = missing || duplicate;

	return (
		<div>
			<div className="grid grid-cols-[1fr_1fr_auto] gap-2">
				<Input
					aria-label={t("config.channel.webhookHeaderName")}
					aria-invalid={invalid}
					aria-describedby={invalid ? errorId : undefined}
					value={draftName}
					onChange={(event) => setDraftName(event.target.value)}
					onBlur={() => {
						if (!invalid && draftName !== name) {
							onChange(name, draftName, value);
						}
					}}
					className="h-8 font-mono text-xs"
				/>
				<Input
					aria-label={t("config.channel.webhookHeaderValue", { name })}
					type="password"
					autoComplete="off"
					value={value}
					onChange={(event) => onChange(name, name, event.target.value)}
					className="h-8 text-xs"
				/>
				<Button
					type="button"
					variant="outline"
					size="sm"
					onClick={() => onChange(name, "", "")}
				>
					{t("config.channel.webhookRemoveHeader")}
				</Button>
			</div>
			{invalid ? (
				<p id={errorId} className="mt-1 text-xs text-destructive">
					{t(
						duplicate
							? "config.channel.webhookHeaderDuplicate"
							: "config.channel.webhookHeaderRequired",
					)}
				</p>
			) : null}
		</div>
	);
}

function WebhookTextField({
	id,
	label,
	value,
	onChange,
}: {
	id: string;
	label: string;
	value: string;
	onChange: (value: string) => void;
}) {
	return (
		<div className="grid gap-1.5 sm:grid-cols-[7rem_minmax(0,1fr)] sm:items-center">
			<label htmlFor={id} className="text-xs text-muted-foreground">
				{label}
			</label>
			<Input
				id={id}
				value={value}
				onChange={(event) => onChange(event.target.value)}
				className="h-8 font-mono text-xs"
			/>
		</div>
	);
}
