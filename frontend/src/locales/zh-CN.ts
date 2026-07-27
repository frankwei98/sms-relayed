import type { en } from "./en";

type TranslationShape<T> = {
	[K in keyof T]: T[K] extends string ? string : TranslationShape<T[K]>;
};

export const zhCN = {
	nav: {
		sms: "短信",
	},
} satisfies TranslationShape<typeof en>;
