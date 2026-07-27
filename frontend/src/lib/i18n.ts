import i18n from "i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import { initReactI18next } from "react-i18next";
import { en } from "#/locales/en";
import { zhCN } from "#/locales/zh-CN";

export const DEFAULT_NAMESPACE = "translation";
export const LANGUAGE_STORAGE_KEY = "sms-relayed.locale";
export const supportedLanguages = ["en", "zh-CN"] as const;

export type SupportedLanguage = (typeof supportedLanguages)[number];

export const resources = {
	en: {
		[DEFAULT_NAMESPACE]: en,
	},
	"zh-CN": {
		[DEFAULT_NAMESPACE]: zhCN,
	},
} as const;

export function normalizeLanguage(language: string): SupportedLanguage {
	return language.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

function updateDocumentLanguage(language: string) {
	document.documentElement.lang = normalizeLanguage(language);
}

i18n.on("languageChanged", updateDocumentLanguage);

export const initPromise = i18n
	.use(LanguageDetector)
	.use(initReactI18next)
	.init({
		resources,
		defaultNS: DEFAULT_NAMESPACE,
		fallbackLng: "en",
		supportedLngs: supportedLanguages,
		detection: {
			order: ["localStorage", "navigator"],
			lookupLocalStorage: LANGUAGE_STORAGE_KEY,
			caches: ["localStorage"],
			convertDetectedLanguage: normalizeLanguage,
		},
		interpolation: {
			escapeValue: false,
		},
		react: {
			useSuspense: false,
		},
		returnNull: false,
	});

export default i18n;
