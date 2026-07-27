import "i18next";
import type { DEFAULT_NAMESPACE, resources } from "#/lib/i18n";

declare module "i18next" {
	interface CustomTypeOptions {
		defaultNS: typeof DEFAULT_NAMESPACE;
		resources: (typeof resources)["en"];
		returnNull: false;
	}
}
