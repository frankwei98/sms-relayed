import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, test } from "vitest";

const projectRoot = resolve(__dirname, "..");

describe("mobile viewport behavior", () => {
	test("disables mobile zoom in the viewport metadata", () => {
		const html = readFileSync(resolve(projectRoot, "index.html"), "utf8");

		expect(html).toContain(
			'name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no"',
		);
	});

	test("prevents page scrolling on mobile viewports", () => {
		const styles = readFileSync(resolve(projectRoot, "src/styles.css"), "utf8");

		expect(styles).toContain("@media (max-width: 767px)");
		expect(styles).toContain("overflow: hidden;");
		expect(styles).toContain("touch-action: pan-x pan-y;");
	});
});
