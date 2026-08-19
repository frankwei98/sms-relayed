import { useCallback, useMemo, useRef, useState } from "react";
import { ApiRequestError } from "#/lib/api";
import {
	type ConfigDocument,
	type ConfigPreviewResponse,
	type ConfigSaveResponse,
	checkConfig,
	previewConfig,
	saveConfig,
} from "#/lib/config-api";
import type { AppConfig } from "#/lib/config-model";
import { CONFIG_SECTIONS, type ConfigSection } from "./config-sections";

export type ConfigCheckState =
	| { status: "idle" }
	| { status: "checking" }
	| { status: "passed" }
	| { status: "failed"; message: string };

export type ConfigPreviewState =
	| { status: "closed" }
	| { status: "loading"; snapshot: AppConfig }
	| {
			status: "ready";
			snapshot: AppConfig;
			response: ConfigPreviewResponse;
			saving: boolean;
			saveError?: string;
	  }
	| {
			status: "error";
			snapshot: AppConfig;
			message: string;
			conflict: boolean;
	  };

function sortedValue(value: unknown): unknown {
	if (Array.isArray(value)) {
		return value.map(sortedValue);
	}
	if (value && typeof value === "object") {
		return Object.fromEntries(
			Object.entries(value as Record<string, unknown>)
				.toSorted(([left], [right]) => left.localeCompare(right))
				.map(([key, entry]) => [key, sortedValue(entry)]),
		);
	}
	return value;
}

function structurallyEqual(left: unknown, right: unknown): boolean {
	return (
		JSON.stringify(sortedValue(left)) === JSON.stringify(sortedValue(right))
	);
}

function sectionValue(config: AppConfig, section: ConfigSection): unknown {
	switch (section) {
		case "device":
			return config.app;
		case "sms":
			return config.sms;
		case "forwarding":
			return {
				forward: config.forward,
				delivery: config.delivery,
				channels: config.channels,
			};
		case "api":
			return config.api;
		case "timeouts":
			return config.http;
		case "retention":
			return config.retention;
	}
}

export function useConfigDraft(initialDocument: ConfigDocument) {
	const [baseline, setBaseline] = useState<AppConfig>(() =>
		structuredClone(initialDocument.config),
	);
	const [draft, setDraft] = useState<AppConfig>(() =>
		structuredClone(initialDocument.config),
	);
	const [baseRevision, setBaseRevision] = useState(initialDocument.revision);
	const [restartRequired, setRestartRequired] = useState(
		initialDocument.restartRequired,
	);
	const [check, setCheck] = useState<ConfigCheckState>({ status: "idle" });
	const [preview, setPreview] = useState<ConfigPreviewState>({
		status: "closed",
	});
	const draftVersion = useRef(0);
	const previewGeneration = useRef(0);

	const dirtySections = useMemo(() => {
		const dirty = new Set<ConfigSection>();
		for (const section of CONFIG_SECTIONS) {
			if (
				!structurallyEqual(
					sectionValue(baseline, section),
					sectionValue(draft, section),
				)
			) {
				dirty.add(section);
			}
		}
		return dirty;
	}, [baseline, draft]);

	const isDirty = dirtySections.size > 0;

	const updateDraft = useCallback((next: AppConfig) => {
		draftVersion.current += 1;
		setDraft(next);
		setCheck({ status: "idle" });
	}, []);

	const updatePath = useCallback((path: string, value: unknown) => {
		draftVersion.current += 1;
		setDraft((current) => {
			const next = structuredClone(current);
			const keys = path.split(".");
			let object = next as unknown as Record<string, unknown>;
			for (let index = 0; index < keys.length - 1; index += 1) {
				object = object[keys[index]] as Record<string, unknown>;
			}
			object[keys.at(-1) as string] = value;
			return next;
		});
		setCheck({ status: "idle" });
	}, []);

	const runCheck = useCallback(async () => {
		const version = draftVersion.current;
		const snapshot = structuredClone(draft);
		setCheck({ status: "checking" });
		try {
			await checkConfig(snapshot);
			if (draftVersion.current === version) {
				setCheck({ status: "passed" });
			}
		} catch (error) {
			if (draftVersion.current === version) {
				setCheck({
					status: "failed",
					message: (error as Error).message,
				});
			}
		}
	}, [draft]);

	const openPreview = useCallback(async () => {
		const generation = previewGeneration.current + 1;
		previewGeneration.current = generation;
		const snapshot = structuredClone(draft);
		setPreview({ status: "loading", snapshot });
		try {
			const response = await previewConfig(snapshot, baseRevision);
			if (previewGeneration.current !== generation) {
				return;
			}
			setPreview({
				status: "ready",
				snapshot,
				response,
				saving: false,
			});
		} catch (error) {
			if (previewGeneration.current !== generation) {
				return;
			}
			setPreview({
				status: "error",
				snapshot,
				message: (error as Error).message,
				conflict:
					error instanceof ApiRequestError && error.code === "config_changed",
			});
		}
	}, [baseRevision, draft]);

	const closePreview = useCallback(() => {
		previewGeneration.current += 1;
		setPreview({ status: "closed" });
	}, []);

	const confirmSave =
		useCallback(async (): Promise<ConfigSaveResponse | null> => {
			if (preview.status !== "ready" || preview.saving) {
				return null;
			}
			const { snapshot, response } = preview;
			setPreview({ ...preview, saving: true, saveError: undefined });
			try {
				const result = await saveConfig(
					snapshot,
					response.base_revision,
					response.candidate_revision,
					response.password_change_pending,
				);
				setBaseline(structuredClone(snapshot));
				setDraft(structuredClone(snapshot));
				setBaseRevision(result.revision);
				setRestartRequired(result.requires_restart);
				draftVersion.current += 1;
				setCheck({ status: "idle" });
				setPreview({ status: "closed" });
				return result;
			} catch (error) {
				if (
					error instanceof ApiRequestError &&
					(error.code === "config_changed" ||
						error.code === "config_preview_changed")
				) {
					setPreview({
						status: "error",
						snapshot,
						message: error.message,
						conflict: true,
					});
				} else {
					setPreview({
						...preview,
						saving: false,
						saveError: (error as Error).message,
					});
				}
				return null;
			}
		}, [preview]);

	const reset = useCallback((document: ConfigDocument) => {
		setBaseline(structuredClone(document.config));
		setDraft(structuredClone(document.config));
		setBaseRevision(document.revision);
		setRestartRequired(document.restartRequired);
		draftVersion.current += 1;
		previewGeneration.current += 1;
		setCheck({ status: "idle" });
		setPreview({ status: "closed" });
	}, []);

	return {
		baseline,
		draft,
		baseRevision,
		restartRequired,
		check,
		preview,
		dirtySections,
		isDirty,
		updateDraft,
		updatePath,
		runCheck,
		openPreview,
		closePreview,
		confirmSave,
		reset,
	};
}
