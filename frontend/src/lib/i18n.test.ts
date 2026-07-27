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

	test("dynamic interpolation works in both languages", () => {
		const enT = i18n.getFixedT("en");
		const zhT = i18n.getFixedT("zh-CN");

		expect(enT("modem.lastChecked", { time: "now" })).toBe("Last checked now");
		expect(zhT("modem.lastChecked", { time: "now" })).toBe("上次检查 now");
	});

	test("pluralization keys exist in both languages", () => {
		const enT = i18n.getFixedT("en");
		const zhT = i18n.getFixedT("zh-CN");

		expect(enT("forwarding.attempts.title", { count: 1 })).toBe(
			"Latest 1 attempt",
		);
		expect(enT("forwarding.attempts.title", { count: 5 })).toBe(
			"Latest 5 attempts",
		);
		expect(zhT("forwarding.attempts.title", { count: 1 })).toBe(
			"最新 1 条尝试",
		);
		expect(zhT("forwarding.attempts.title", { count: 5 })).toBe(
			"最新 5 条尝试",
		);
	});

	test("common UI strings are translated in zh-CN", () => {
		const zhT = i18n.getFixedT("zh-CN");

		// Verify that common action labels are translated
		expect(zhT("common.refresh")).toBe("刷新");
		expect(zhT("common.cancel")).toBe("取消");
		expect(zhT("common.save")).toBe("保存");

		// Verify nav items are translated
		expect(zhT("nav.sms")).toBe("短信");
		expect(zhT("nav.modem")).toBe("调制解调器");
		expect(zhT("nav.forwarding")).toBe("转发");
		expect(zhT("nav.config")).toBe("配置");
	});

	test("login page in zh-CN has no hardcoded English action text", () => {
		const zhT = i18n.getFixedT("zh-CN");

		// The login button text should be translated
		expect(zhT("login.login")).toBe("登录");
		expect(zhT("login.password")).toBe("密码");

		// The login notice should be translated
		expect(zhT("login.notice.configSavedRestart")).toContain("配置已保存");

		// English action labels should NOT appear in zh-CN
		expect(zhT("login.login")).not.toBe("Login");
		expect(zhT("login.password")).not.toBe("Password");
	});

	test("config action labels are translated in zh-CN", () => {
		const zhT = i18n.getFixedT("zh-CN");

		expect(zhT("config.action.save")).toBe("保存");
		expect(zhT("config.action.check")).toBe("检查");
		expect(zhT("config.action.restart")).toBe("重启");
	});

	test("modem action labels are translated in zh-CN", () => {
		const zhT = i18n.getFixedT("zh-CN");

		expect(zhT("modem.actions.enable")).toBe("启用");
		expect(zhT("modem.actions.disable")).toBe("禁用");
		expect(zhT("modem.dangerZone.reset")).toBe("重置调制解调器");
	});

	test("language switcher labels are available", () => {
		const enT = i18n.getFixedT("en");
		const zhT = i18n.getFixedT("zh-CN");

		expect(enT("language.en")).toBe("English");
		expect(enT("language.zhCN")).toBe("简体中文");
		expect(zhT("language.en")).toBe("English");
		expect(zhT("language.zhCN")).toBe("简体中文");
	});

	test("forwarding outcome strings differ between languages", () => {
		const enT = i18n.getFixedT("en");
		const zhT = i18n.getFixedT("zh-CN");

		expect(enT("forwarding.outcome.success")).toBe("Success");
		expect(zhT("forwarding.outcome.success")).toBe("成功");
		expect(enT("forwarding.outcome.noAttempts")).toBe("No attempts");
		expect(zhT("forwarding.outcome.noAttempts")).toBe("无尝试");
	});

	test("all resource keys are present in both languages", () => {
		const enBundle = i18n.getResourceBundle("en", "translation");
		const zhBundle = i18n.getResourceBundle("zh-CN", "translation");

		// Compare dot-notation keys, stripping i18next plural suffixes
		function normalizeKey(key: string) {
			return key.replace(/_(one|other|plural)$/, "");
		}

		const enKeys = new Set([...collectKeys(enBundle)].map(normalizeKey));
		const zhKeys = new Set([...collectKeys(zhBundle)].map(normalizeKey));

		// Every base key in English should exist in zh-CN
		const missingInZh = [...enKeys].filter((k) => !zhKeys.has(k));
		const missingInEn = [...zhKeys].filter((k) => !enKeys.has(k));

		expect(
			missingInZh,
			`Keys missing in zh-CN: ${missingInZh.join(", ")}`,
		).toEqual([]);
		expect(
			missingInEn,
			`Keys missing in en: ${missingInEn.join(", ")}`,
		).toEqual([]);
	});
});

function collectKeys(obj: Record<string, unknown>, prefix = ""): Set<string> {
	const keys = new Set<string>();
	for (const [key, value] of Object.entries(obj)) {
		const fullKey = prefix ? `${prefix}.${key}` : key;
		if (value && typeof value === "object") {
			const subKeys = collectKeys(value as Record<string, unknown>, fullKey);
			for (const subKey of subKeys) keys.add(subKey);
		} else {
			keys.add(fullKey);
		}
	}
	return keys;
}
