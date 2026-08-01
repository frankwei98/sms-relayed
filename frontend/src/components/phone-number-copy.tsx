import { Check, Copy } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { copyText } from "#/lib/clipboard";

export function PhoneNumberCopy({ phoneNumber }: { phoneNumber: string }) {
	const [result, setResult] = useState<"idle" | "copied" | "failed">("idle");
	const resetTimer = useRef<number | undefined>(undefined);
	const latestRequest = useRef(0);
	const { t } = useTranslation();

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
				aria-label={t("phoneCopy.ariaLabel")}
				onClick={copy}
			>
				{result === "copied" ? <Check /> : <Copy />}
				{result === "copied"
					? t("phoneCopy.copied")
					: result === "failed"
						? t("phoneCopy.copyFailed")
						: t("phoneCopy.copy")}
			</Button>
			<output className="sr-only" aria-live="polite">
				{result === "copied"
					? t("phoneCopy.srCopied")
					: result === "failed"
						? t("phoneCopy.srFailed")
						: ""}
			</output>
		</>
	);
}
