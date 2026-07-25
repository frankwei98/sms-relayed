import { Check, Copy } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "#/components/ui/button";

export function PhoneNumberCopy({ phoneNumber }: { phoneNumber: string }) {
	const [result, setResult] = useState<"idle" | "copied" | "failed">("idle");
	const resetTimer = useRef<number | undefined>(undefined);
	const latestRequest = useRef(0);

	async function copy() {
		const request = ++latestRequest.current;
		const nextResult = (await copyText(phoneNumber)) ? "copied" : "failed";
		if (request !== latestRequest.current) return;

		setResult(nextResult);
		window.clearTimeout(resetTimer.current);
		resetTimer.current = window.setTimeout(() => {
			if (request === latestRequest.current) setResult("idle");
		}, 2000);
	}

	useEffect(
		() => () => {
			window.clearTimeout(resetTimer.current);
		},
		[],
	);

	return (
		<>
			<Button
				type="button"
				size="sm"
				variant="ghost"
				aria-label="Copy phone number"
				onClick={copy}
			>
				{result === "copied" ? <Check /> : <Copy />}
				{result === "copied"
					? "Copied"
					: result === "failed"
						? "Copy failed"
						: "Copy"}
			</Button>
			<output className="sr-only" aria-live="polite">
				{result === "copied"
					? "Phone number copied"
					: result === "failed"
						? "Phone number copy failed"
						: ""}
			</output>
		</>
	);
}

async function copyText(value: string) {
	try {
		if (navigator.clipboard?.writeText) {
			await navigator.clipboard.writeText(value);
			return true;
		}
	} catch {
		// Fall back for non-secure HTTP deployments.
	}

	const textarea = document.createElement("textarea");
	textarea.value = value;
	textarea.setAttribute("readonly", "");
	textarea.style.position = "fixed";
	textarea.style.opacity = "0";
	document.body.appendChild(textarea);
	textarea.select();

	try {
		return document.execCommand?.("copy") === true;
	} catch {
		return false;
	} finally {
		textarea.remove();
	}
}
