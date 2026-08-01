import { LoaderCircle, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "#/components/ui/dialog";
import type { Message } from "#/lib/api";

export function DeleteMessageDialog({
	message,
	pending,
	onOpenChange,
	onConfirm,
}: {
	message: Message | null;
	pending: boolean;
	onOpenChange: (open: boolean) => void;
	onConfirm: () => void;
}) {
	const { t } = useTranslation();
	return (
		<Dialog open={Boolean(message)} onOpenChange={onOpenChange}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>{t("messages.deleteMessage.title")}</DialogTitle>
					<DialogDescription>
						{t("messages.deleteMessage.description")}
					</DialogDescription>
				</DialogHeader>
				{message ? (
					<blockquote className="max-h-28 overflow-auto rounded-2xl bg-muted px-4 py-3 text-sm leading-relaxed break-words">
						{message.body}
					</blockquote>
				) : null}
				<DialogFooter>
					<DialogClose
						disabled={pending}
						render={<Button type="button" variant="outline" />}
					>
						{t("common.cancel")}
					</DialogClose>
					<Button
						type="button"
						variant="destructive"
						disabled={pending}
						onClick={onConfirm}
					>
						{pending ? <LoaderCircle className="animate-spin" /> : <Trash2 />}
						{t("messages.deleteMessage.confirm")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function useDesktopContextMenus() {
	const mediaQuery = "(pointer: coarse)";
	const [enabled, setEnabled] = useState(() =>
		typeof window.matchMedia === "function"
			? !window.matchMedia(mediaQuery).matches
			: true,
	);

	useEffect(() => {
		if (typeof window.matchMedia !== "function") return;
		const query = window.matchMedia(mediaQuery);
		const update = () => setEnabled(!query.matches);
		query.addEventListener("change", update);
		return () => query.removeEventListener("change", update);
	}, []);

	return enabled;
}
