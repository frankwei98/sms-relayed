import { useEffect, useState } from "react";
import { ChannelEditor } from "#/components/config/channel-editor";
import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import { Switch } from "#/components/ui/switch";
import { apiFetch } from "#/lib/api";
import type { AppConfig } from "#/lib/config-model";

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
		if (Array.isArray(v)) return v.join(", ");
		if (typeof v === "number" || typeof v === "boolean") return String(v);
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
						value={s("sms.ignore_storage")}
						onChange={(e) =>
							update(
								"sms.ignore_storage",
								e.target.value
									.split(",")
									.map((s) => s.trim())
									.filter(Boolean),
							)
						}
						className="h-8 flex-1"
					/>
				</div>
				<div className="flex items-start gap-2">
					<span className="w-32 text-sm pt-1">code_keywords</span>
					<Input
						value={s("sms.code_keywords")}
						onChange={(e) =>
							update(
								"sms.code_keywords",
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
				<h3 className="font-medium">api</h3>
				<div className="flex items-center gap-2">
					<span className="w-32 text-sm">enabled</span>
					<Switch
						checked={config.api.enabled}
						onCheckedChange={(c: boolean) => update("api.enabled", c)}
					/>
				</div>
				{["api.bind", "api.port", "api.password", "api.database_path"].map(
					(k) => (
						<div key={k} className="flex items-center gap-2">
							<span className="w-32 text-sm">{k.split(".")[1]}</span>
							<Input
								value={s(k)}
								onChange={(e) =>
									update(
										k,
										k.endsWith("port")
											? Number(e.target.value)
											: e.target.value,
									)
								}
								className="h-8 flex-1"
								type={k === "api.password" ? "password" : "text"}
							/>
						</div>
					),
				)}
				<div className="flex items-center gap-2">
					<span className="w-32 text-sm">enable_ipv6</span>
					<Switch
						checked={config.api.enable_ipv6}
						onCheckedChange={(c: boolean) => update("api.enable_ipv6", c)}
					/>
				</div>
			</section>

			<section className="space-y-2">
				<h3 className="font-medium">forward</h3>
				<div className="flex items-start gap-2">
					<span className="w-32 text-sm pt-1">enabled</span>
					<Input
						value={s("forward.enabled")}
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
				<ChannelEditor config={config} onUpdate={setConfig} />
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
