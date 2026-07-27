// @vitest-environment jsdom

import { afterEach, describe, expect, test } from "vitest";
import i18n, { LANGUAGE_STORAGE_KEY, normalizeLanguage } from "#/lib/i18n";

afterEach(async () => {
	localStorage.clear();
	await i18n.changeLanguage("en");
});

describe("i18n foundation", () => {
	test.each([
		["en", "en"],
		["en-US", "en"],
		["zh", "zh-CN"],
		["zh-SG", "zh-CN"],
		["zh-Hant-TW", "zh-CN"],
	])("normalizes %s to %s", (input, expected) => {
		expect(normalizeLanguage(input)).toBe(expected);
	});

	test("ships matching English and Simplified Chinese resources", () => {
		expect(i18n.getFixedT("en")("nav.sms")).toBe("SMS");
		expect(i18n.getFixedT("zh-CN")("nav.sms")).toBe("短信");
	});

	test("persists language changes and updates the document language", async () => {
		await i18n.changeLanguage("zh-CN");

		expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe("zh-CN");
		expect(document.documentElement.lang).toBe("zh-CN");
	});
});
