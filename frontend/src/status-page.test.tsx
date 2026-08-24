// @vitest-environment jsdom

import {
	cleanup,
	createEvent,
	fireEvent,
	render,
	screen,
} from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { type StatusSection, StatusTabs } from "#/routes/status";

function ControlledStatusTabs({
	initialSection = "modem",
	onChange,
}: {
	initialSection?: StatusSection;
	onChange?: (section: StatusSection) => void;
}) {
	const [section, setSection] = useState<StatusSection>(initialSection);

	return (
		<StatusTabs
			activeSection={section}
			onChange={(nextSection) => {
				onChange?.(nextSection);
				setSection(nextSection);
			}}
		/>
	);
}

afterEach(() => {
	cleanup();
});

describe("StatusTabs accessibility", () => {
	test("keeps only the active tab in the tab sequence", () => {
		render(<ControlledStatusTabs />);

		const tabs = screen.getAllByRole("tab");
		expect(tabs[0].getAttribute("tabindex")).toBe("0");
		expect(tabs[1].getAttribute("tabindex")).toBe("-1");

		fireEvent.click(tabs[1]);

		expect(tabs[0].getAttribute("tabindex")).toBe("-1");
		expect(tabs[1].getAttribute("tabindex")).toBe("0");
	});

	test.each([
		["ArrowLeft", "modem", "forwarding"],
		["ArrowRight", "modem", "forwarding"],
		["ArrowLeft", "forwarding", "modem"],
		["ArrowRight", "forwarding", "modem"],
		["Home", "forwarding", "modem"],
		["End", "modem", "forwarding"],
	] as const)("handles %s from %s and activates %s", (key, currentSection, expectedSection) => {
		const onChange = vi.fn<(section: StatusSection) => void>();
		render(
			<ControlledStatusTabs
				initialSection={currentSection}
				onChange={onChange}
			/>,
		);

		const tabs = screen.getAllByRole("tab");
		const currentIndex = currentSection === "modem" ? 0 : 1;
		const expectedIndex = expectedSection === "modem" ? 0 : 1;
		const event = createEvent.keyDown(tabs[currentIndex], { key });

		tabs[currentIndex].focus();
		fireEvent(tabs[currentIndex], event);

		expect(event.defaultPrevented).toBe(true);
		expect(onChange).toHaveBeenCalledWith(expectedSection);
		expect(document.activeElement).toBe(tabs[expectedIndex]);
		expect(tabs[currentIndex].getAttribute("tabindex")).toBe("-1");
		expect(tabs[expectedIndex].getAttribute("tabindex")).toBe("0");
	});
});
