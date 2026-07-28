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
	/** Translation key for the label (config.sections.*) */
	labelTKey: string;
	/** Translation key for the description (config.sections.*Description) */
	descriptionTKey: string;
};

export const CONFIG_SECTION_DEFINITIONS: ConfigSectionDefinition[] = [
	{
		id: "device",
		label: "Device",
		description: "Modem identity and object path",
		labelTKey: "config.sections.device",
		descriptionTKey: "config.sections.deviceDescription",
		icon: Smartphone,
	},
	{
		id: "sms",
		label: "SMS",
		description: "Storage filters and code keywords",
		labelTKey: "config.sections.sms",
		descriptionTKey: "config.sections.smsDescription",
		icon: MessageSquareText,
	},
	{
		id: "forwarding",
		label: "Forwarding",
		description: "Delivery workers and channel profiles",
		labelTKey: "config.sections.forwarding",
		descriptionTKey: "config.sections.forwardingDescription",
		icon: RadioTower,
	},
	{
		id: "api",
		label: "Web API",
		description: "Dashboard access and persistence",
		labelTKey: "config.sections.api",
		descriptionTKey: "config.sections.apiDescription",
		icon: Globe2,
	},
	{
		id: "timeouts",
		label: "Timeouts",
		description: "HTTP connection and request limits",
		labelTKey: "config.sections.timeouts",
		descriptionTKey: "config.sections.timeoutsDescription",
		icon: Clock3,
	},
	{
		id: "retention",
		label: "Retention",
		description: "Automatic message cleanup",
		labelTKey: "config.sections.retention",
		descriptionTKey: "config.sections.retentionDescription",
		icon: Database,
	},
];

export function isConfigSection(value: unknown): value is ConfigSection {
	return (
		typeof value === "string" &&
		CONFIG_SECTIONS.includes(value as ConfigSection)
	);
}
