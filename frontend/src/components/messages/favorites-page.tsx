import { Copy, Inbox, LoaderCircle, Star, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
	DeleteMessageDialog,
	useDesktopContextMenus,
} from "#/components/messages/message-context-menu";
import { Badge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuTrigger,
} from "#/components/ui/context-menu";
import { apiFetch, type Message } from "#/lib/api";
import { subscribeEvents } from "#/lib/events";

type FavoritesError = "load" | "update" | "copy" | "delete";

export function FavoritesPage({
	onOpenMessage,
}: {
	onOpenMessage: (phoneNumber: string, messageId: number) => void;
}) {
	const { t, i18n } = useTranslation();
	const [messages, setMessages] = useState<Message[]>([]);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<FavoritesError | null>(null);
	const [notice, setNotice] = useState(false);
	const [deleteTarget, setDeleteTarget] = useState<Message | null>(null);
	const [deletePending, setDeletePending] = useState(false);
	const contextMenusEnabled = useDesktopContextMenus();

	const loadFavorites = useCallback(async () => {
		try {
			const favorites = await apiFetch<Message[]>("/api/messages/favorites");
			setMessages(favorites);
			setError(null);
		} catch {
			setError("load");
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		void loadFavorites();
		return subscribeEvents({
			"message.updated": () => void loadFavorites(),
			"message.deleted": () => void loadFavorites(),
		});
	}, [loadFavorites]);

	useEffect(() => {
		if (!notice) return;
		const timeout = window.setTimeout(() => setNotice(false), 2500);
		return () => window.clearTimeout(timeout);
	}, [notice]);

	const orderedMessages = useMemo(
		() =>
			[...messages].sort((a, b) =>
				(b.favorite_at ?? "").localeCompare(a.favorite_at ?? ""),
			),
		[messages],
	);

	async function removeFavorite(message: Message) {
		try {
			await apiFetch(`/api/messages/${message.id}/unfavorite`, {
				method: "POST",
			});
			setMessages((current) =>
				current.filter((item) => item.id !== message.id),
			);
			setError(null);
		} catch {
			setError("update");
		}
	}

	async function copyMessage(message: Message) {
		try {
			await navigator.clipboard.writeText(message.body);
			setNotice(true);
			setError(null);
		} catch {
			setError("copy");
		}
	}

	async function confirmDelete() {
		if (!deleteTarget || deletePending) return;
		const id = deleteTarget.id;
		setDeletePending(true);
		try {
			await apiFetch(`/api/messages/${id}`, { method: "DELETE" });
			setMessages((current) => current.filter((message) => message.id !== id));
			setDeleteTarget(null);
			setError(null);
		} catch {
			setError("delete");
		} finally {
			setDeletePending(false);
		}
	}

	const locale = i18n.resolvedLanguage ?? i18n.language;
	const errorText = error
		? {
				load: t("favorites.error.load"),
				update: t("messages.error.update"),
				copy: t("messages.error.copy"),
				delete: t("messages.error.delete"),
			}[error]
		: null;

	return (
		<div className="mx-auto w-full max-w-5xl space-y-6 pb-10">
			<header className="flex items-start gap-4">
				<div className="grid size-11 shrink-0 place-items-center rounded-2xl bg-amber-500/10 text-amber-600 dark:text-amber-300">
					<Star className="size-5 fill-current" />
				</div>
				<div>
					<h2 className="text-2xl font-semibold tracking-tight">
						{t("favorites.title")}
					</h2>
					<p className="mt-1 text-sm text-muted-foreground">
						{t("favorites.subtitle", { count: orderedMessages.length })}
					</p>
				</div>
			</header>

			{notice ? (
				<output className="fixed right-4 bottom-4 z-50 rounded-xl bg-foreground px-4 py-2 text-sm text-background shadow-lg">
					{t("messages.notice.copied")}
				</output>
			) : null}

			{errorText ? (
				<div
					role="alert"
					className="flex items-center justify-between gap-3 rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
				>
					<span>{errorText}</span>
					<Button
						type="button"
						variant="ghost"
						size="sm"
						onClick={() => setError(null)}
					>
						{t("common.done")}
					</Button>
				</div>
			) : null}

			{loading ? (
				<div className="grid min-h-64 place-items-center text-muted-foreground">
					<LoaderCircle
						className="animate-spin"
						aria-label={t("favorites.loading")}
					/>
				</div>
			) : orderedMessages.length === 0 ? (
				<div className="grid min-h-64 place-items-center rounded-3xl border border-dashed bg-muted/20 px-6 text-center">
					<div className="max-w-sm">
						<Inbox className="mx-auto size-9 text-muted-foreground" />
						<h3 className="mt-4 font-medium">{t("favorites.emptyTitle")}</h3>
						<p className="mt-1 text-sm text-muted-foreground">
							{t("favorites.emptyDescription")}
						</p>
					</div>
				</div>
			) : (
				<div className="space-y-3">
					{orderedMessages.map((message) => (
						<ContextMenu key={message.id} disabled={!contextMenusEnabled}>
							<ContextMenuTrigger
								render={
									<button
										type="button"
										className="group w-full rounded-3xl border bg-card p-4 text-left shadow-xs transition-colors hover:border-primary/30 hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
										onClick={() =>
											onOpenMessage(message.phone_number, message.id)
										}
									/>
								}
							>
								<div className="flex items-start justify-between gap-4">
									<div className="min-w-0">
										<div className="flex flex-wrap items-center gap-2">
											<span className="font-semibold">
												{message.phone_number}
											</span>
											<Badge variant="secondary">
												{message.direction === "outbound"
													? t("messages.direction.sent")
													: t("messages.direction.inbox")}
											</Badge>
										</div>
										<p className="mt-3 whitespace-pre-wrap text-sm leading-6 break-words">
											{message.body}
										</p>
									</div>
									<Star className="size-4 shrink-0 fill-amber-500 text-amber-500" />
								</div>
								<div className="mt-4 flex flex-wrap gap-x-5 gap-y-1 text-xs text-muted-foreground">
									<span>
										{t("favorites.originalTime")}:{" "}
										{formatDate(message.timestamp, locale)}
									</span>
								</div>
							</ContextMenuTrigger>
							<ContextMenuContent className="w-52">
								<ContextMenuItem onClick={() => void removeFavorite(message)}>
									<Star />
									{t("messages.actions.unfavorite")}
								</ContextMenuItem>
								<ContextMenuItem onClick={() => void copyMessage(message)}>
									<Copy />
									{t("messages.actions.copy")}
								</ContextMenuItem>
								<ContextMenuItem
									variant="destructive"
									disabled={message.delete_blocked}
									title={
										message.delete_blocked
											? t("messages.actions.deleteSendingDisabled")
											: undefined
									}
									onClick={() => setDeleteTarget(message)}
								>
									<Trash2 />
									{t("messages.actions.delete")}
								</ContextMenuItem>
							</ContextMenuContent>
						</ContextMenu>
					))}
				</div>
			)}

			<DeleteMessageDialog
				message={deleteTarget}
				pending={deletePending}
				onOpenChange={(open) => {
					if (!open && !deletePending) setDeleteTarget(null);
				}}
				onConfirm={() => void confirmDelete()}
			/>
		</div>
	);
}

function formatDate(value: string, locale: string) {
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return value;
	return new Intl.DateTimeFormat(locale, {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(date);
}
