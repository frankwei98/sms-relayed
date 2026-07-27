import {
	type ComponentProps,
	cloneElement,
	type ReactElement,
	useEffect,
	useRef,
	useState,
} from "react";
import { useTranslation } from "react-i18next";
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
	const canonicalValue = value.join(", ");
	const [text, setText] = useState(() => canonicalValue);
	const lastValue = useRef(canonicalValue);

	useEffect(() => {
		if (canonicalValue === lastValue.current) return;
		lastValue.current = canonicalValue;
		setText(canonicalValue);
	}, [canonicalValue]);

	return (
		<Input
			{...props}
			value={text}
			onChange={(event) => {
				const next = event.target.value;
				const parsed = arrayInput(next);
				setText(next);
				lastValue.current = parsed.join(", ");
				onValueChange(parsed);
			}}
			onBlur={() => setText(canonicalValue)}
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
	const lastValue = useRef(value);

	useEffect(() => {
		if (value === lastValue.current) return;
		lastValue.current = value;
		setText(String(value));
	}, [value]);

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
				if (Number.isFinite(parsed)) {
					lastValue.current = parsed;
					onValueChange(parsed);
				}
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
	const { t } = useTranslation();
	return (
		<>
			<SectionHeading
				title={t("config.fields.device.sectionTitle")}
				description={t("config.fields.device.sectionDescription")}
			/>
			<Field
				id="app-device-name"
				label={t("config.fields.device.deviceName")}
				description={t("config.fields.device.deviceNameDescription")}
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
				label={t("config.fields.device.modemPath")}
				description={t("config.fields.device.modemPathDescription")}
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
	const { t } = useTranslation();
	return (
		<>
			<SectionHeading
				title={t("config.fields.sms.sectionTitle")}
				description={t("config.fields.sms.sectionDescription")}
			/>
			<Field
				id="sms-ignore-storage"
				label={t("config.fields.sms.ignoredStorage")}
				description={t("config.fields.sms.ignoredStorageDescription")}
			>
				<ArrayInput
					id="sms-ignore-storage"
					value={config.sms.ignore_storage}
					onValueChange={(value) => onPathChange("sms.ignore_storage", value)}
				/>
			</Field>
			<Field
				id="sms-code-keywords"
				label={t("config.fields.sms.codeKeywords")}
				description={t("config.fields.sms.codeKeywordsDescription")}
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
	const { t } = useTranslation();
	return (
		<>
			<SectionHeading
				title={t("config.fields.forwarding.sectionTitle")}
				description={t("config.fields.forwarding.sectionDescription")}
			/>
			<Field
				id="delivery-concurrency"
				label={t("config.fields.forwarding.concurrency")}
				description={t("config.fields.forwarding.concurrencyDescription")}
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
	const { t } = useTranslation();
	return (
		<>
			<SectionHeading
				title={t("config.fields.api.sectionTitle")}
				description={t("config.fields.api.sectionDescription")}
			/>
			<Field
				id="api-enabled"
				label={t("config.fields.api.enableApi")}
				description={t("config.fields.api.enableApiDescription")}
			>
				<Switch
					id="api-enabled"
					checked={config.api.enabled}
					onCheckedChange={(checked) => onPathChange("api.enabled", checked)}
				/>
			</Field>
			<Field id="api-bind" label={t("config.fields.api.bindAddress")}>
				<Input
					id="api-bind"
					className="font-mono text-xs"
					value={config.api.bind}
					onChange={(event) => onPathChange("api.bind", event.target.value)}
				/>
			</Field>
			<Field
				id="api-port"
				label={t("config.fields.api.port")}
				description={t("config.fields.api.portDescription")}
			>
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
				label={t("config.fields.api.ipv6")}
				description={t("config.fields.api.ipv6Description")}
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
				label={t("config.fields.api.password")}
				description={t("config.fields.api.passwordDescription")}
			>
				<Input
					id="api-password"
					type="password"
					autoComplete="new-password"
					value={config.api.password}
					onChange={(event) => onPathChange("api.password", event.target.value)}
				/>
			</Field>
			<Field id="api-database" label={t("config.fields.api.databasePath")}>
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
	const { t } = useTranslation();
	return (
		<>
			<SectionHeading
				title={t("config.fields.timeouts.sectionTitle")}
				description={t("config.fields.timeouts.sectionDescription")}
			/>
			<Field
				id="http-connect-timeout"
				label={t("config.fields.timeouts.connectTimeout")}
				description={t("config.fields.timeouts.connectTimeoutDescription")}
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
			<Field
				id="http-request-timeout"
				label={t("config.fields.timeouts.requestTimeout")}
			>
				<NumberInput
					id="http-request-timeout"
					min={1}
					value={config.http.request_timeout_secs}
					onValueChange={(value) =>
						onPathChange("http.request_timeout_secs", value)
					}
				/>
			</Field>
			<Field
				id="shell-timeout"
				label={t("config.fields.timeouts.shellTimeout")}
			>
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
	const { t } = useTranslation();
	return (
		<>
			<SectionHeading
				title={t("config.fields.retention.sectionTitle")}
				description={t("config.fields.retention.sectionDescription")}
			/>
			<Field
				id="retention-enabled"
				label={t("config.fields.retention.enableCleanup")}
			>
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
				label={t("config.fields.retention.maxAge")}
				description={t("config.fields.retention.maxAgeDescription")}
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
				label={t("config.fields.retention.batchSize")}
				description={t("config.fields.retention.batchSizeDescription")}
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
