import type { LucideIcon } from "lucide-react";
import {
	Clock3,
	Database,
	Globe2,
	MessageSquareText,
	RadioTower,
	Smartphone,
} from "lucide-react";

export const CONFIG_SECTIONS = [
	"device",
	"sms",
	"forwarding",
	"api",
	"timeouts",
	"retention",
] as const;

export type ConfigSection = (typeof CONFIG_SECTIONS)[number];

export type ConfigSectionDefinition = {
	id: ConfigSection;
	label: string;
	description: string;
	icon: LucideIcon;
};

export const CONFIG_SECTION_DEFINITIONS: ConfigSectionDefinition[] = [
	{
		id: "device",
		label: "Device",
		description: "Modem identity and object path",
		icon: Smartphone,
	},
	{
		id: "sms",
		label: "SMS",
		description: "Storage filters and code keywords",
		icon: MessageSquareText,
	},
	{
		id: "forwarding",
		label: "Forwarding",
		description: "Delivery workers and channel profiles",
		icon: RadioTower,
	},
	{
		id: "api",
		label: "Web API",
		description: "Dashboard access and persistence",
		icon: Globe2,
	},
	{
		id: "timeouts",
		label: "Timeouts",
		description: "HTTP and shell execution limits",
		icon: Clock3,
	},
	{
		id: "retention",
		label: "Retention",
		description: "Automatic message cleanup",
		icon: Database,
	},
];

export function isConfigSection(value: unknown): value is ConfigSection {
	return (
		typeof value === "string" &&
		CONFIG_SECTIONS.includes(value as ConfigSection)
	);
}
