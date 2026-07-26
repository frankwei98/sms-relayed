import {
	type ComponentProps,
	cloneElement,
	type ReactElement,
	useState,
} from "react";
import { ChannelEditor } from "#/components/config/channel-editor";
import { Input } from "#/components/ui/input";
import { Switch } from "#/components/ui/switch";
import type { AppConfig } from "#/lib/config-model";
import type { ConfigSection } from "./config-sections";

type SectionEditorProps = {
	config: AppConfig;
	onConfigChange: (config: AppConfig) => void;
	onPathChange: (path: string, value: unknown) => void;
};

type FieldProps = {
	id: string;
	label: string;
	description?: string;
	children: ReactElement<{ "aria-describedby"?: string }>;
};

function Field({ id, label, description, children }: FieldProps) {
	const descriptionId = description ? `${id}-description` : undefined;
	const describedBy = [children.props["aria-describedby"], descriptionId]
		.filter(Boolean)
		.join(" ");
	return (
		<div className="grid gap-2 border-b py-4 last:border-b-0 md:grid-cols-[13rem_minmax(0,1fr)] md:gap-6">
			<div>
				<label htmlFor={id} className="text-sm font-medium">
					{label}
				</label>
				{description ? (
					<p
						id={descriptionId}
						className="mt-1 text-xs leading-relaxed text-muted-foreground"
					>
						{description}
					</p>
				) : null}
			</div>
			<div className="min-w-0">
				{cloneElement(children, {
					"aria-describedby": describedBy || undefined,
				})}
			</div>
		</div>
	);
}

function SectionHeading({
	title,
	description,
}: {
	title: string;
	description: string;
}) {
	return (
		<div className="border-b pb-5">
			<h2 className="text-xl font-semibold tracking-tight">{title}</h2>
			<p className="mt-1 max-w-2xl text-sm leading-relaxed text-muted-foreground">
				{description}
			</p>
		</div>
	);
}

function arrayInput(next: string): string[] {
	return next
		.split(",")
		.map((entry) => entry.trim())
		.filter(Boolean);
}

type ArrayInputProps = Omit<
	ComponentProps<typeof Input>,
	"value" | "onChange" | "onBlur"
> & {
	value: string[];
	onValueChange: (value: string[]) => void;
};

function ArrayInput({ value, onValueChange, ...props }: ArrayInputProps) {
	const [text, setText] = useState(() => value.join(", "));

	return (
		<Input
			{...props}
			value={text}
			onChange={(event) => {
				const next = event.target.value;
				setText(next);
				onValueChange(arrayInput(next));
			}}
			onBlur={() => setText(value.join(", "))}
		/>
	);
}

type NumberInputProps = Omit<
	ComponentProps<typeof Input>,
	"value" | "onChange" | "onBlur" | "type"
> & {
	value: number;
	onValueChange: (value: number) => void;
};

function NumberInput({ value, onValueChange, ...props }: NumberInputProps) {
	const [text, setText] = useState(() => String(value));

	return (
		<Input
			{...props}
			type="number"
			value={text}
			onChange={(event) => {
				const next = event.target.value;
				setText(next);
				if (next === "") return;
				const parsed = Number(next);
				if (Number.isFinite(parsed)) onValueChange(parsed);
			}}
			onBlur={() => setText(String(value))}
		/>
	);
}

export function ConfigSectionEditor({
	section,
	config,
	onConfigChange,
	onPathChange,
}: SectionEditorProps & { section: ConfigSection }) {
	return (
		<div className="mx-auto w-full max-w-4xl p-4 md:p-8">
			{section === "device" ? (
				<DeviceSection config={config} onPathChange={onPathChange} />
			) : null}
			{section === "sms" ? (
				<SmsSection config={config} onPathChange={onPathChange} />
			) : null}
			{section === "forwarding" ? (
				<ForwardingSection
					config={config}
					onConfigChange={onConfigChange}
					onPathChange={onPathChange}
				/>
			) : null}
			{section === "api" ? (
				<ApiSection config={config} onPathChange={onPathChange} />
			) : null}
			{section === "timeouts" ? (
				<TimeoutsSection config={config} onPathChange={onPathChange} />
			) : null}
			{section === "retention" ? (
				<RetentionSection config={config} onPathChange={onPathChange} />
			) : null}
		</div>
	);
}

function DeviceSection({
	config,
	onPathChange,
}: Pick<SectionEditorProps, "config" | "onPathChange">) {
	return (
		<>
			<SectionHeading
				title="Device"
				description="Identify this relay and select the ModemManager object that receives and sends messages."
			/>
			<Field
				id="app-device-name"
				label="Device name"
				description="Included in forwarding payloads so downstream channels can identify the source."
			>
				<Input
					id="app-device-name"
					value={config.app.device_name}
					onChange={(event) =>
						onPathChange("app.device_name", event.target.value)
					}
				/>
			</Field>
			<Field
				id="app-modem-path"
				label="Modem object path"
				description="Must be a ModemManager path under /org/freedesktop/ModemManager1/Modem/."
			>
				<Input
					id="app-modem-path"
					className="font-mono text-xs"
					value={config.app.modem_path}
					onChange={(event) =>
						onPathChange("app.modem_path", event.target.value)
					}
				/>
			</Field>
		</>
	);
}

function SmsSection({
	config,
	onPathChange,
}: Pick<SectionEditorProps, "config" | "onPathChange">) {
	return (
		<>
			<SectionHeading
				title="SMS"
				description="Control which modem storage locations are ignored and which phrases identify verification-code messages."
			/>
			<Field
				id="sms-ignore-storage"
				label="Ignored storage"
				description="Comma-separated storage identifiers, such as sm."
			>
				<ArrayInput
					id="sms-ignore-storage"
					value={config.sms.ignore_storage}
					onValueChange={(value) => onPathChange("sms.ignore_storage", value)}
				/>
			</Field>
			<Field
				id="sms-code-keywords"
				label="Code keywords"
				description="Comma-separated, case-insensitive phrases used to recognize verification codes."
			>
				<ArrayInput
					id="sms-code-keywords"
					value={config.sms.code_keywords}
					onValueChange={(value) => onPathChange("sms.code_keywords", value)}
				/>
			</Field>
		</>
	);
}

function ForwardingSection({
	config,
	onConfigChange,
	onPathChange,
}: SectionEditorProps) {
	return (
		<>
			<SectionHeading
				title="Forwarding"
				description="Configure delivery concurrency, channel credentials, and the named profiles that receive inbound messages."
			/>
			<Field
				id="delivery-concurrency"
				label="Concurrent deliveries"
				description="Number of forwarding jobs processed at once. Valid range: 1–16."
			>
				<NumberInput
					id="delivery-concurrency"
					min={1}
					max={16}
					value={config.delivery.concurrency}
					onValueChange={(value) => onPathChange("delivery.concurrency", value)}
				/>
			</Field>
			<div className="pt-6">
				<ChannelEditor config={config} onUpdate={onConfigChange} />
			</div>
		</>
	);
}

function ApiSection({
	config,
	onPathChange,
}: Pick<SectionEditorProps, "config" | "onPathChange">) {
	return (
		<>
			<SectionHeading
				title="Web API"
				description="Control dashboard availability, listener addresses, authentication, and the message database."
			/>
			<Field
				id="api-enabled"
				label="Enable Web API"
				description="Disabling the API removes access to this dashboard after restart."
			>
				<Switch
					id="api-enabled"
					checked={config.api.enabled}
					onCheckedChange={(checked) => onPathChange("api.enabled", checked)}
				/>
			</Field>
			<Field id="api-bind" label="Bind address">
				<Input
					id="api-bind"
					className="font-mono text-xs"
					value={config.api.bind}
					onChange={(event) => onPathChange("api.bind", event.target.value)}
				/>
			</Field>
			<Field id="api-port" label="Port" description="Valid range: 1–65535.">
				<NumberInput
					id="api-port"
					min={1}
					max={65535}
					value={config.api.port}
					onValueChange={(value) => onPathChange("api.port", value)}
				/>
			</Field>
			<Field
				id="api-ipv6"
				label="IPv6 companion"
				description="Also listen on a safe IPv6 companion address when one can be inferred."
			>
				<Switch
					id="api-ipv6"
					checked={config.api.enable_ipv6}
					onCheckedChange={(checked) =>
						onPathChange("api.enable_ipv6", checked)
					}
				/>
			</Field>
			<Field
				id="api-password"
				label="Password"
				description="Changing this value saves and schedules restart in one step, then signs out every session."
			>
				<Input
					id="api-password"
					type="password"
					autoComplete="new-password"
					value={config.api.password}
					onChange={(event) => onPathChange("api.password", event.target.value)}
				/>
			</Field>
			<Field id="api-database" label="Database path">
				<Input
					id="api-database"
					className="font-mono text-xs"
					value={config.api.database_path}
					onChange={(event) =>
						onPathChange("api.database_path", event.target.value)
					}
				/>
			</Field>
		</>
	);
}

function TimeoutsSection({
	config,
	onPathChange,
}: Pick<SectionEditorProps, "config" | "onPathChange">) {
	return (
		<>
			<SectionHeading
				title="Timeouts"
				description="Bound connection setup, provider requests, and shell-profile execution. All values are seconds."
			/>
			<Field
				id="http-connect-timeout"
				label="Connect timeout"
				description="Must be positive and no greater than the request timeout."
			>
				<NumberInput
					id="http-connect-timeout"
					min={1}
					value={config.http.connect_timeout_secs}
					onValueChange={(value) =>
						onPathChange("http.connect_timeout_secs", value)
					}
				/>
			</Field>
			<Field id="http-request-timeout" label="Request timeout">
				<NumberInput
					id="http-request-timeout"
					min={1}
					value={config.http.request_timeout_secs}
					onValueChange={(value) =>
						onPathChange("http.request_timeout_secs", value)
					}
				/>
			</Field>
			<Field id="shell-timeout" label="Shell timeout">
				<NumberInput
					id="shell-timeout"
					min={1}
					value={config.http.shell_timeout_secs}
					onValueChange={(value) =>
						onPathChange("http.shell_timeout_secs", value)
					}
				/>
			</Field>
		</>
	);
}

function RetentionSection({
	config,
	onPathChange,
}: Pick<SectionEditorProps, "config" | "onPathChange">) {
	return (
		<>
			<SectionHeading
				title="Retention"
				description="Remove old terminal messages in bounded batches while preserving messages with active deliveries."
			/>
			<Field id="retention-enabled" label="Enable cleanup">
				<Switch
					id="retention-enabled"
					checked={config.retention.enabled}
					onCheckedChange={(checked) =>
						onPathChange("retention.enabled", checked)
					}
				/>
			</Field>
			<Field
				id="retention-max-age"
				label="Maximum age"
				description="Messages older than this many days become eligible for cleanup."
			>
				<NumberInput
					id="retention-max-age"
					min={1}
					value={config.retention.max_age_days}
					onValueChange={(value) =>
						onPathChange("retention.max_age_days", value)
					}
				/>
			</Field>
			<Field
				id="retention-batch-size"
				label="Batch size"
				description="Maximum rows removed by one cleanup pass."
			>
				<NumberInput
					id="retention-batch-size"
					min={1}
					value={config.retention.batch_size}
					onValueChange={(value) => onPathChange("retention.batch_size", value)}
				/>
			</Field>
		</>
	);
}
