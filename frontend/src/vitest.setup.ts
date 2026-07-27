// Only initialize i18n in jsdom environments where browser APIs are available.
if (typeof window !== "undefined" && typeof navigator !== "undefined") {
	try {
		const i18nModule = await import("#/lib/i18n");
		// Ensure i18n is initialized before tests run
		await i18nModule.initPromise;
	} catch {
		// Silently skip i18n init in non-browser test environments.
	}
}
