import { useEffect, useState } from "react";
import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import { type AppConfig, apiFetch } from "#/lib/api";

export function ConfigEditor() {
	const [config, setConfig] = useState<AppConfig | null>(null);
	const [result, setResult] = useState("");
	const [loading, setLoading] = useState(true);

	useEffect(() => {
		apiFetch<AppConfig>("/api/config")
			.then(setConfig)
			.catch((e) => setResult(`Error: ${e.message}`))
			.finally(() => setLoading(false));
	}, []);

	if (loading) return <p>Loading config...</p>;
	if (!config) return <p>Failed to load config.</p>;

	const update = (path: string, value: unknown) => {
		setConfig((prev) => {
			if (!prev) return prev;
			const clone = structuredClone(prev);
			const keys = path.split(".");
			let obj: Record<string, unknown> = clone as Record<string, unknown>;
			for (let i = 0; i < keys.length - 1; i++) {
				obj = obj[keys[i]] as Record<string, unknown>;
			}
			obj[keys[keys.length - 1]] = value;
			return clone;
		});
	};

	async function handleCheck() {
		setResult("");
		try {
			await apiFetch("/api/config/check", {
				method: "POST",
				body: JSON.stringify(config),
			});
			setResult("Config is valid.");
		} catch (e: unknown) {
			setResult(`Invalid: ${(e as Error).message}`);
		}
	}

	async function handleSave() {
		setResult("");
		try {
			await apiFetch<{ requires_restart: boolean }>("/api/config", {
				method: "PUT",
				body: JSON.stringify(config),
			});
			setResult("Saved. Restart required.");
		} catch (e: unknown) {
			setResult(`Save failed: ${(e as Error).message}`);
		}
	}

	async function handleRestart() {
		setResult("");
		try {
			await apiFetch("/api/service/restart", { method: "POST" });
			setResult("Restart scheduled.");
		} catch (e: unknown) {
			setResult(`Restart failed: ${(e as Error).message}`);
		}
	}

	const s = (key: string) => {
		const keys = key.split(".");
		let v: unknown = config;
		for (const k of keys) {
			v = (v as Record<string, unknown>)?.[k];
		}
		return (v ?? "") as string;
	};

	return (
		<div className="space-y-6">
			<h2 className="text-lg font-semibold">Configuration</h2>

			<section className="space-y-2">
				<h3 className="font-medium">app</h3>
				{["app.device_name", "app.modem_path"].map((k) => (
					<div key={k} className="flex items-center gap-2">
						<span className="w-32 text-sm">{k.split(".")[1]}</span>
						<Input
							value={s(k)}
							onChange={(e) => update(k, e.target.value)}
							className="h-8 flex-1"
						/>
					</div>
				))}
			</section>

			<section className="space-y-2">
				<h3 className="font-medium">sms</h3>
				<div className="flex items-start gap-2">
					<span className="w-32 text-sm pt-1">ignore_storage</span>
					<Input
						value={s("sms.ignore_storage").replace(/[[\]"]/g, "")}
						onChange={(e) =>
							update(
								"sms.ignore_storage",
								e.target.value.split(",").map((s) => s.trim()),
							)
						}
						className="h-8 flex-1"
					/>
				</div>
				<div className="flex items-start gap-2">
					<span className="w-32 text-sm pt-1">code_keywords</span>
					<Input
						value={s("sms.code_keywords").replace(/[[\]"]/g, "")}
						onChange={(e) =>
							update(
								"sms.code_keywords",
								e.target.value.split(",").map((s) => s.trim()),
							)
						}
						className="h-8 flex-1"
					/>
				</div>
			</section>

			<section className="space-y-2">
				<h3 className="font-medium">api</h3>
				{[
					"api.enabled",
					"api.bind",
					"api.port",
					"api.enable_ipv6",
					"api.password",
					"api.database_path",
				].map((k) => (
					<div key={k} className="flex items-center gap-2">
						<span className="w-32 text-sm">{k.split(".")[1]}</span>
						{k === "api.enabled" || k === "api.enable_ipv6" ? (
							<input
								type="checkbox"
								checked={s(k) === "true"}
								onChange={(e) => update(k, e.target.checked)}
							/>
						) : k === "api.port" ? (
							<Input
								value={s(k)}
								onChange={(e) => update(k, Number(e.target.value))}
								className="h-8 flex-1"
							/>
						) : (
							<Input
								value={s(k)}
								onChange={(e) => update(k, e.target.value)}
								className="h-8 flex-1"
								type={k === "api.password" ? "password" : "text"}
							/>
						)}
					</div>
				))}
			</section>

			<section className="space-y-2">
				<h3 className="font-medium">forward</h3>
				<div className="flex items-start gap-2">
					<span className="w-32 text-sm pt-1">enabled</span>
					<Input
						value={s("forward.enabled").replace(/[[\]"]/g, "")}
						onChange={(e) =>
							update(
								"forward.enabled",
								e.target.value
									.split(",")
									.map((s) => s.trim())
									.filter(Boolean),
							)
						}
						className="h-8 flex-1"
					/>
				</div>
			</section>

			<section className="space-y-2">
				<h3 className="font-medium">channels</h3>
				{(
					[
						"bark",
						"telegram",
						"pushplus",
						"wecom",
						"dingtalk",
						"shell",
					] as const
				).map((channel) => (
					<div key={channel} className="rounded border p-3">
						<h4 className="text-sm font-medium capitalize">{channel}</h4>
						<div className="flex flex-wrap gap-2 mt-1">
							{Object.keys(config.channels[channel] ?? {}).map((name) => (
								<span
									key={name}
									className="rounded bg-muted px-2 py-0.5 text-xs"
								>
									{name}
								</span>
							))}
							{Object.keys(config.channels[channel] ?? {}).length === 0 && (
								<span className="text-xs text-muted-foreground">
									No profiles
								</span>
							)}
						</div>
					</div>
				))}
			</section>

			<div className="flex gap-2">
				<Button onClick={handleCheck}>Check</Button>
				<Button onClick={handleSave}>Save</Button>
				<Button variant="destructive" onClick={handleRestart}>
					Restart
				</Button>
			</div>

			{result && <pre className="rounded bg-muted p-2 text-sm">{result}</pre>}
		</div>
	);
}
