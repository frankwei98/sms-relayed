import dayjs from "dayjs";
import relativeTime from "dayjs/plugin/relativeTime";
import {
	Archive,
	CheckCheck,
	ChevronLeft,
	Download,
	Filter,
	Inbox,
	LoaderCircle,
	MessageCircle,
	MoreHorizontal,
	Plus,
	Search,
	Send,
	Trash2,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { PhoneNumberCopy } from "#/components/phone-number-copy";
import { Badge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import { Checkbox } from "#/components/ui/checkbox";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "#/components/ui/dialog";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "#/components/ui/dropdown-menu";
import { Input } from "#/components/ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "#/components/ui/select";
import { Textarea } from "#/components/ui/textarea";
import { apiFetch, type ConversationSummary, type Message } from "#/lib/api";
import { subscribeEvents } from "#/lib/events";
import { fetchModemStatus } from "#/lib/modem-api";
import { cn } from "#/lib/utils";

dayjs.extend(relativeTime);
dayjs.locale(navigator.language.toLowerCase());

const ALL_DIRECTIONS = "all-directions";
const ALL_STATUSES = "all-statuses";
const MESSAGE_PAGE_SIZE = 10;
const SERVER_MESSAGE_PAGE_LIMIT = 500;
const FALLBACK_REFRESH_INTERVAL_MS = 30_000;
const SQLITE_TIMESTAMP_PATTERN =
	/^(?:\d{4}-\d{2}-\d{2}|\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))$/;
type MessageOperationError =
	| "markRead"
	| "update"
	| "delete"
	| "loadOlder"
	| "refresh";

function createIdempotencyKey() {
	const bytes = crypto.getRandomValues(new Uint8Array(16));
	return `web-${Array.from(bytes, (byte) =>
		byte.toString(16).padStart(2, "0"),
	).join("")}`;
}

function messageDisplayTimestamp(
	message: Pick<Message, "created_at" | "timestamp">,
) {
	return !SQLITE_TIMESTAMP_PATTERN.test(message.timestamp) ||
		Number.isNaN(Date.parse(message.timestamp))
		? message.created_at
		: message.timestamp;
}

function messageFromEvent(payload: unknown) {
	if (!payload || typeof payload !== "object" || !("payload" in payload)) {
		return null;
	}
	const message = payload.payload;
	if (
		!message ||
		typeof message !== "object" ||
		!("id" in message) ||
		!("phone_number" in message)
	) {
		return null;
	}
	return message as Message;
}

function sortMessagesChronologically(messages: Message[]) {
	return [...messages].reverse();
}

function mergeMessagesChronologically(current: Message[], incoming: Message[]) {
	const incomingIds = new Set(incoming.map((message) => message.id));
	return [
		...sortMessagesChronologically(incoming),
		...current.filter((message) => !incomingIds.has(message.id)),
	];
}

export function MessageConsole() {
	const [conversations, setConversations] = useState<ConversationSummary[]>([]);
	const [messages, setMessages] = useState<Message[]>([]);
	const [selectedPhone, setSelectedPhone] = useState<string | null>(null);
	const [isComposingNew, setIsComposingNew] = useState(false);
	const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
	const [selectionMode, setSelectionMode] = useState(false);
	const [q, setQ] = useState("");
	const [direction, setDirection] = useState(ALL_DIRECTIONS);
	const [statusFilter, setStatusFilter] = useState(ALL_STATUSES);
	const [unreadOnly, setUnreadOnly] = useState(false);
	const [phoneNumber, setPhoneNumber] = useState("");
	const [body, setBody] = useState("");
	const [sending, setSending] = useState(false);
	const [sendFailed, setSendFailed] = useState(false);
	const [hasOlderMessages, setHasOlderMessages] = useState(false);
	const [loadingOlderMessages, setLoadingOlderMessages] = useState(false);
	const [ownNumber, setOwnNumber] = useState<string | null>(null);
	const [conversationLoadFailed, setConversationLoadFailed] = useState(false);
	const [operationError, setOperationError] =
		useState<MessageOperationError | null>(null);
	const markingReadPhonesRef = useRef<Set<string>>(new Set());
	const hasLoadedConversationsRef = useRef(false);
	const loadedMessageCountRef = useRef(0);
	const pendingSendRef = useRef<{
		phoneNumber: string;
		body: string;
		idempotencyKey: string;
	} | null>(null);
	const messageWindowGenerationRef = useRef(0);
	const messageReloadVersionRef = useRef(0);
	const messageRefreshEpochRef = useRef(0);
	const oldestLoadedMessageIdRef = useRef<number | null>(null);

	const resetMessageWindow = useCallback(() => {
		messageWindowGenerationRef.current += 1;
		messageReloadVersionRef.current += 1;
		messageRefreshEpochRef.current += 1;
		loadedMessageCountRef.current = 0;
		oldestLoadedMessageIdRef.current = null;
		setMessages([]);
		setHasOlderMessages(false);
		setLoadingOlderMessages(false);
	}, []);

	const buildParams = useCallback(
		(phone?: string | null) => {
			const p = new URLSearchParams();
			if (phone) p.set("phone_number", phone);
			if (q) p.set("q", q);
			if (direction !== ALL_DIRECTIONS) p.set("direction", direction);
			if (statusFilter !== ALL_STATUSES) p.set("status", statusFilter);
			if (unreadOnly) p.set("unread", "true");
			return p;
		},
		[q, direction, statusFilter, unreadOnly],
	);

	const fetchMessagePage = useCallback(
		async (
			phone: string,
			before?: Pick<Message, "created_at" | "id" | "timestamp">,
			limit = MESSAGE_PAGE_SIZE,
		) => {
			const p = buildParams(phone);
			p.set("limit", String(limit));
			if (before) {
				p.set("before_timestamp", before.timestamp);
				p.set("before_id", String(before.id));
			}
			return apiFetch<Message[]>(`/api/messages?${p.toString()}`);
		},
		[buildParams],
	);

	const fetchMessageWindow = useCallback(
		async (phone: string, limit: number) => {
			const window: Message[] = [];
			let before: Message | undefined;
			while (window.length < limit) {
				const pageLimit = Math.min(
					SERVER_MESSAGE_PAGE_LIMIT,
					limit - window.length,
				);
				const page = await fetchMessagePage(phone, before, pageLimit);
				window.push(...page);
				if (page.length < pageLimit) break;
				before = page.at(-1);
				if (!before) break;
			}
			return window;
		},
		[fetchMessagePage],
	);

	const loadMessagesForPhone = useCallback(
		async (phone: string, limit = MESSAGE_PAGE_SIZE, createdMessages = 0) => {
			const generation = messageWindowGenerationRef.current;
			const reloadVersion = ++messageReloadVersionRef.current;
			const loadedAtStart = loadedMessageCountRef.current;
			const baseRequestedLimit = limit + createdMessages;
			let requestedLimit = baseRequestedLimit;
			let data: Message[];
			while (true) {
				data = await fetchMessageWindow(phone, requestedLimit);
				if (
					generation !== messageWindowGenerationRef.current ||
					reloadVersion !== messageReloadVersionRef.current
				) {
					return;
				}
				const expandedLimit =
					baseRequestedLimit +
					Math.max(0, loadedMessageCountRef.current - loadedAtStart);
				if (expandedLimit <= requestedLimit) break;
				requestedLimit = expandedLimit;
			}
			const serverWindowFilled = data.length === requestedLimit;
			if (createdMessages > 0 && oldestLoadedMessageIdRef.current !== null) {
				const boundaryIndex = data.findIndex(
					(message) => message.id === oldestLoadedMessageIdRef.current,
				);
				if (boundaryIndex >= 0) data = data.slice(0, boundaryIndex + 1);
			}
			const ordered = sortMessagesChronologically(data);
			messageRefreshEpochRef.current += 1;
			loadedMessageCountRef.current = ordered.length;
			oldestLoadedMessageIdRef.current = ordered[0]?.id ?? null;
			setMessages(ordered);
			setHasOlderMessages(serverWindowFilled);
		},
		[fetchMessageWindow],
	);

	const loadMessages = useCallback(async () => {
		if (!selectedPhone) {
			resetMessageWindow();
			return;
		}
		await loadMessagesForPhone(selectedPhone);
	}, [loadMessagesForPhone, resetMessageWindow, selectedPhone]);

	const loadOlderMessages = useCallback(async () => {
		if (
			!selectedPhone ||
			loadingOlderMessages ||
			!hasOlderMessages ||
			messages.length === 0
		) {
			return;
		}

		setOperationError(null);
		setLoadingOlderMessages(true);
		const generation = messageWindowGenerationRef.current;
		const refreshEpoch = messageRefreshEpochRef.current;
		try {
			const data = await fetchMessagePage(selectedPhone, messages[0]);
			if (
				generation !== messageWindowGenerationRef.current ||
				refreshEpoch !== messageRefreshEpochRef.current
			) {
				return;
			}
			setMessages((current) => {
				const merged = mergeMessagesChronologically(current, data);
				loadedMessageCountRef.current = merged.length;
				oldestLoadedMessageIdRef.current = merged[0]?.id ?? null;
				return merged;
			});
			setHasOlderMessages(data.length === MESSAGE_PAGE_SIZE);
		} catch (err) {
			setOperationError("loadOlder");
			console.error(err);
		} finally {
			setLoadingOlderMessages(false);
		}
	}, [
		fetchMessagePage,
		hasOlderMessages,
		loadingOlderMessages,
		messages,
		selectedPhone,
	]);

	const loadConversations = useCallback(async () => {
		try {
			const data = await apiFetch<ConversationSummary[]>("/api/conversations");
			hasLoadedConversationsRef.current = true;
			setConversationLoadFailed(false);
			setConversations(data);
		} catch (error) {
			if (hasLoadedConversationsRef.current) {
				setOperationError("refresh");
			} else {
				setConversationLoadFailed(true);
			}
			throw error;
		}
	}, []);

	const reloadActiveViews = useCallback(
		async (createdMessages = 0) => {
			const messageReload = selectedPhone
				? loadMessagesForPhone(
						selectedPhone,
						Math.max(MESSAGE_PAGE_SIZE, loadedMessageCountRef.current),
						createdMessages,
					)
				: Promise.resolve();
			await Promise.all([messageReload, loadConversations()]);
		},
		[loadConversations, loadMessagesForPhone, selectedPhone],
	);

	useEffect(() => {
		void loadConversations().catch(() => {});
	}, [loadConversations]);

	useEffect(() => {
		fetchModemStatus()
			.then((status) => setOwnNumber(status.modem.own_number))
			.catch(() => {});
	}, []);

	useEffect(() => {
		void loadMessages().catch(() => setOperationError("refresh"));
	}, [loadMessages]);

	const refreshTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const pendingCreatedMessagesRef = useRef(0);

	const scheduleRefresh = useCallback(
		(createdMessage?: Message | null) => {
			if (createdMessage?.phone_number === selectedPhone) {
				pendingCreatedMessagesRef.current += 1;
			}
			if (refreshTimeoutRef.current) clearTimeout(refreshTimeoutRef.current);
			refreshTimeoutRef.current = setTimeout(() => {
				const additionalMessages = pendingCreatedMessagesRef.current;
				pendingCreatedMessagesRef.current = 0;
				void reloadActiveViews(additionalMessages).catch((err) => {
					setOperationError("refresh");
					console.error(err);
				});
				refreshTimeoutRef.current = null;
			}, 100);
		},
		[reloadActiveViews, selectedPhone],
	);

	useEffect(() => {
		const unsub = subscribeEvents({
			"message.created": (payload) =>
				scheduleRefresh(messageFromEvent(payload)),
			"message.updated": () => scheduleRefresh(),
			"message.deleted": () => scheduleRefresh(),
			"message.read_state_changed": () => scheduleRefresh(),
			"conversation.read": () => scheduleRefresh(),
		});
		const refreshInterval = window.setInterval(
			() => scheduleRefresh(),
			FALLBACK_REFRESH_INTERVAL_MS,
		);
		return () => {
			unsub();
			window.clearInterval(refreshInterval);
			if (refreshTimeoutRef.current) clearTimeout(refreshTimeoutRef.current);
			pendingCreatedMessagesRef.current = 0;
		};
	}, [scheduleRefresh]);

	useEffect(() => {
		if (
			selectedPhone &&
			conversations.length > 0 &&
			!conversations.some((c) => c.phone_number === selectedPhone)
		) {
			setSelectedPhone(null);
			setIsComposingNew(false);
		}
	}, [conversations, selectedPhone]);

	useEffect(() => {
		if (!selectedPhone || isComposingNew) return;
		if (markingReadPhonesRef.current.has(selectedPhone)) return;
		const hasUnread = messages.some(
			(message) =>
				message.phone_number === selectedPhone &&
				message.direction === "inbound" &&
				message.read_at === null,
		);
		if (!hasUnread) return;

		const phone = selectedPhone;
		markingReadPhonesRef.current.add(phone);

		apiFetch(`/api/conversations/${encodeURIComponent(phone)}/read`, {
			method: "POST",
		})
			.then(async () => {
				await reloadActiveViews();
			})
			.catch((err) => {
				setOperationError("markRead");
				console.error(err);
			})
			.finally(() => {
				markingReadPhonesRef.current.delete(phone);
			});
	}, [isComposingNew, messages, reloadActiveViews, selectedPhone]);

	const filteredConversations = useMemo(() => {
		const needle = q.trim().toLowerCase();
		return conversations.filter((conversation) => {
			const last = conversation.last_message;
			if (unreadOnly && conversation.unread_count === 0) return false;
			if (direction !== ALL_DIRECTIONS && last.direction !== direction)
				return false;
			if (statusFilter !== ALL_STATUSES && last.status !== statusFilter) {
				return false;
			}
			if (!needle) return true;
			return (
				conversation.phone_number.toLowerCase().includes(needle) ||
				last.body.toLowerCase().includes(needle)
			);
		});
	}, [conversations, direction, q, statusFilter, unreadOnly]);

	const selectedConversation = useMemo(
		() =>
			conversations.find(
				(conversation) => conversation.phone_number === selectedPhone,
			) ?? null,
		[conversations, selectedPhone],
	);

	const activePhone = isComposingNew ? phoneNumber : selectedPhone;
	const hasThreadOpen = Boolean(selectedPhone || isComposingNew);

	function selectConversation(phone: string) {
		if (!isComposingNew && phone === selectedPhone) return;
		resetMessageWindow();
		setSelectedPhone(phone);
		setPhoneNumber(phone);
		setIsComposingNew(false);
		setSelectionMode(false);
		setSelectedIds(new Set());
	}

	function startNewMessage() {
		setSelectedPhone(null);
		setPhoneNumber("");
		resetMessageWindow();
		setIsComposingNew(true);
		setSelectionMode(false);
		setSelectedIds(new Set());
	}

	function closeMobileThread() {
		resetMessageWindow();
		setIsComposingNew(false);
		setSelectedPhone(null);
		setSelectionMode(false);
		setSelectedIds(new Set());
	}

	function toggleSelect(id: number) {
		setSelectedIds((prev) => {
			const next = new Set(prev);
			if (next.has(id)) next.delete(id);
			else next.add(id);
			return next;
		});
	}

	async function handleSend() {
		const recipient = activePhone?.trim();
		if (!recipient || !body.trim()) return;
		const pendingSend = pendingSendRef.current;
		const idempotencyKey =
			pendingSend?.phoneNumber === recipient && pendingSend.body === body
				? pendingSend.idempotencyKey
				: createIdempotencyKey();
		pendingSendRef.current = {
			phoneNumber: recipient,
			body,
			idempotencyKey,
		};
		setSendFailed(false);
		setSending(true);
		let sendCompleted = false;
		try {
			await apiFetch<Message>("/api/messages/send", {
				method: "POST",
				headers: { "Idempotency-Key": idempotencyKey },
				body: JSON.stringify({ phone_number: recipient, body }),
			});
			sendCompleted = true;
			pendingSendRef.current = null;
			setBody("");
			setSelectedPhone(recipient);
			setPhoneNumber(recipient);
			setIsComposingNew(false);
			const windowSize =
				recipient === selectedPhone
					? Math.max(MESSAGE_PAGE_SIZE, loadedMessageCountRef.current + 1)
					: MESSAGE_PAGE_SIZE;
			await Promise.all([
				loadMessagesForPhone(recipient, windowSize),
				loadConversations(),
			]);
		} catch (err) {
			if (sendCompleted) {
				setOperationError("refresh");
			} else {
				setSendFailed(true);
			}
			console.error(err);
		} finally {
			setSending(false);
		}
	}

	async function handleMarkConversationRead() {
		if (!selectedPhone) return;
		setOperationError(null);
		try {
			await apiFetch(
				`/api/conversations/${encodeURIComponent(selectedPhone)}/read`,
				{
					method: "POST",
				},
			);
			await reloadActiveViews();
		} catch (err) {
			setOperationError("markRead");
			console.error(err);
		}
	}

	async function handleMarkSelected(read: boolean) {
		const ids = Array.from(selectedIds);
		if (ids.length === 0) return;
		setOperationError(null);
		try {
			await Promise.all(
				ids.map((id) =>
					apiFetch(`/api/messages/${id}/${read ? "read" : "unread"}`, {
						method: "POST",
					}),
				),
			);
			setSelectedIds(new Set());
			setSelectionMode(false);
			await reloadActiveViews();
		} catch (err) {
			setOperationError("update");
			console.error(err);
		}
	}

	async function handleDeleteSelected() {
		if (selectedIds.size === 0) return;
		setOperationError(null);
		try {
			await apiFetch("/api/messages/delete", {
				method: "POST",
				body: JSON.stringify({ ids: Array.from(selectedIds) }),
			});
			setSelectedIds(new Set());
			setSelectionMode(false);
			await reloadActiveViews();
		} catch (err) {
			setOperationError("delete");
			console.error(err);
		}
	}

	function exportMessages(format: "csv" | "json") {
		const p = buildParams(selectedPhone);
		window.location.href = `/api/messages/export?format=${format}&${p.toString()}`;
	}

	return (
		<div className="flex h-full min-h-0 flex-col bg-background">
			{operationError ? (
				<MessageOperationErrorBanner
					error={operationError}
					onDismiss={() => setOperationError(null)}
				/>
			) : null}
			<div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden md:grid-cols-[22rem_minmax(0,1fr)] md:gap-4">
				<section
					className={cn(
						"flex min-h-0 flex-col border-border bg-background md:flex",
						hasThreadOpen && "hidden md:flex",
					)}
				>
					<ConversationListHeader
						ownNumber={ownNumber}
						query={q}
						onQueryChange={setQ}
						onNewMessage={startNewMessage}
						filters={
							<FilterDialog
								direction={direction}
								setDirection={setDirection}
								statusFilter={statusFilter}
								setStatusFilter={setStatusFilter}
								unreadOnly={unreadOnly}
								setUnreadOnly={setUnreadOnly}
								onExportCsv={() => exportMessages("csv")}
								onExportJson={() => exportMessages("json")}
							/>
						}
					/>
					<ConversationList
						conversations={filteredConversations}
						selectedPhone={selectedPhone}
						onSelect={selectConversation}
						loadFailed={conversationLoadFailed}
						onRetry={() => void loadConversations().catch(() => {})}
					/>
				</section>

				<section
					className={cn(
						"min-h-0 bg-muted/30 md:block md:rounded-[min(var(--radius-4xl),28px)] md:border md:bg-card",
						!hasThreadOpen && "hidden md:block",
					)}
				>
					<ThreadPanel
						conversation={selectedConversation}
						messages={messages}
						hasOlderMessages={hasOlderMessages}
						loadingOlderMessages={loadingOlderMessages}
						isComposingNew={isComposingNew}
						phoneNumber={phoneNumber}
						setPhoneNumber={setPhoneNumber}
						body={body}
						setBody={setBody}
						sending={sending}
						sendFailed={sendFailed}
						selectionMode={selectionMode}
						setSelectionMode={setSelectionMode}
						selectedIds={selectedIds}
						onToggleSelect={toggleSelect}
						onLoadOlderMessages={loadOlderMessages}
						onBack={closeMobileThread}
						onSend={handleSend}
						onMarkConversationRead={handleMarkConversationRead}
						onMarkSelectedRead={() => handleMarkSelected(true)}
						onMarkSelectedUnread={() => handleMarkSelected(false)}
						onDeleteSelected={handleDeleteSelected}
					/>
				</section>
			</div>
		</div>
	);
}

function ConversationListHeader({
	ownNumber,
	query,
	onQueryChange,
	onNewMessage,
	filters,
}: {
	ownNumber: string | null;
	query: string;
	onQueryChange: (value: string) => void;
	onNewMessage: () => void;
	filters: ReactNode;
}) {
	const { t } = useTranslation();
	return (
		<div className="shrink-0 border-b bg-background/95 px-4 py-3 backdrop-blur md:rounded-t-[min(var(--radius-4xl),28px)] md:border-x md:border-t">
			<div className="flex items-center justify-between gap-3">
				<div>
					<p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
						{t("nav.sms")}
					</p>
					<h2 className="font-heading text-2xl font-semibold tracking-normal">
						{t("messages.title")}
					</h2>
					{ownNumber ? (
						<div className="flex items-center gap-1 text-sm text-muted-foreground">
							<span>{t("messages.sim", { number: ownNumber })}</span>
							<PhoneNumberCopy phoneNumber={ownNumber} />
						</div>
					) : null}
				</div>
				<div className="flex items-center gap-2">
					{filters}
					<Button
						type="button"
						size="icon"
						variant="default"
						aria-label={t("messages.aria.newMessage")}
						onClick={onNewMessage}
					>
						<Plus />
					</Button>
				</div>
			</div>
			<div className="relative mt-3">
				<Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
				<Input
					value={query}
					onChange={(event) => onQueryChange(event.target.value)}
					placeholder={t("messages.search.placeholder")}
					className="h-10 bg-muted pl-9"
				/>
			</div>
		</div>
	);
}

function FilterDialog({
	direction,
	setDirection,
	statusFilter,
	setStatusFilter,
	unreadOnly,
	setUnreadOnly,
	onExportCsv,
	onExportJson,
}: {
	direction: string;
	setDirection: (value: string) => void;
	statusFilter: string;
	setStatusFilter: (value: string) => void;
	unreadOnly: boolean;
	setUnreadOnly: (value: boolean) => void;
	onExportCsv: () => void;
	onExportJson: () => void;
}) {
	const { t } = useTranslation();
	return (
		<Dialog>
			<DialogTrigger
				render={
					<Button
						type="button"
						size="icon"
						variant="outline"
						aria-label={t("messages.aria.filters")}
					/>
				}
			>
				<Filter />
			</DialogTrigger>
			<DialogContent className="gap-5">
				<DialogHeader>
					<DialogTitle>{t("messages.filter.title")}</DialogTitle>
					<DialogDescription>
						{t("messages.filter.description")}
					</DialogDescription>
				</DialogHeader>
				<div className="grid gap-4">
					<div className="grid gap-1.5">
						<span className="text-xs font-medium text-muted-foreground">
							{t("messages.filter.direction")}
						</span>
						<Select
							value={direction}
							onValueChange={(value) => {
								if (value) setDirection(value);
							}}
						>
							<SelectTrigger className="w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value={ALL_DIRECTIONS}>
									{t("messages.filter.allDirections")}
								</SelectItem>
								<SelectItem value="inbound">
									{t("messages.filter.inbound")}
								</SelectItem>
								<SelectItem value="outbound">
									{t("messages.filter.outbound")}
								</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="grid gap-1.5">
						<span className="text-xs font-medium text-muted-foreground">
							{t("messages.filter.status")}
						</span>
						<Select
							value={statusFilter}
							onValueChange={(value) => {
								if (value) setStatusFilter(value);
							}}
						>
							<SelectTrigger className="w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value={ALL_STATUSES}>
									{t("messages.filter.allStatuses")}
								</SelectItem>
								<SelectItem value="received">
									{t("messages.filter.received")}
								</SelectItem>
								<SelectItem value="sending">
									{t("messages.filter.sending")}
								</SelectItem>
								<SelectItem value="sent">
									{t("messages.filter.sent")}
								</SelectItem>
								<SelectItem value="failed">
									{t("messages.filter.failed")}
								</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="flex items-center justify-between rounded-2xl bg-muted/70 px-3 py-2">
						<span className="text-sm font-medium">
							{t("messages.filter.unreadOnly")}
						</span>
						<Checkbox
							checked={unreadOnly}
							onCheckedChange={(checked) => setUnreadOnly(checked === true)}
						/>
					</div>
				</div>
				<DialogFooter className="grid grid-cols-2 sm:flex">
					<Button type="button" variant="outline" onClick={onExportCsv}>
						<Download />
						{t("messages.filter.exportCsv")}
					</Button>
					<Button type="button" variant="outline" onClick={onExportJson}>
						<Archive />
						{t("messages.filter.exportJson")}
					</Button>
					<DialogClose render={<Button type="button" variant="secondary" />}>
						{t("messages.filter.done")}
					</DialogClose>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function ConversationList({
	conversations,
	selectedPhone,
	onSelect,
	loadFailed,
	onRetry,
}: {
	conversations: ConversationSummary[];
	selectedPhone: string | null;
	onSelect: (phone: string) => void;
	loadFailed: boolean;
	onRetry: () => void;
}) {
	const { t } = useTranslation();
	if (loadFailed) {
		return (
			<div
				role="alert"
				className="grid flex-1 place-items-center px-6 text-center"
			>
				<div className="max-w-64 space-y-3">
					<p className="font-medium text-destructive">
						{t("messages.error.load")}
					</p>
					<Button type="button" variant="outline" onClick={onRetry}>
						{t("common.retry")}
					</Button>
				</div>
			</div>
		);
	}
	if (conversations.length === 0) {
		return (
			<div className="grid flex-1 place-items-center px-6 text-center">
				<div className="max-w-64 space-y-3">
					<div className="mx-auto grid size-12 place-items-center rounded-2xl bg-muted text-muted-foreground">
						<Inbox className="size-5" />
					</div>
					<div>
						<p className="font-medium">
							{t("messages.conversationList.empty")}
						</p>
						<p className="mt-1 text-sm text-muted-foreground">
							{t("messages.conversationList.emptyDescription")}
						</p>
					</div>
				</div>
			</div>
		);
	}

	return (
		<div className="min-h-0 flex-1 overflow-y-auto px-2 py-2 md:rounded-b-[min(var(--radius-4xl),28px)] md:border-x md:border-b">
			<div className="space-y-1">
				{conversations.map((conversation) => (
					<ConversationCard
						key={conversation.phone_number}
						conversation={conversation}
						active={conversation.phone_number === selectedPhone}
						onClick={() => onSelect(conversation.phone_number)}
					/>
				))}
			</div>
		</div>
	);
}

function MessageOperationErrorBanner({
	error,
	onDismiss,
}: {
	error: MessageOperationError;
	onDismiss: () => void;
}) {
	const { t } = useTranslation();
	const messages = {
		markRead: t("messages.error.markRead"),
		update: t("messages.error.update"),
		delete: t("messages.error.delete"),
		loadOlder: t("messages.error.loadOlder"),
		refresh: t("messages.error.refresh"),
	} satisfies Record<MessageOperationError, string>;

	return (
		<div
			role="alert"
			className="flex shrink-0 items-center justify-between gap-3 border-b border-destructive/30 bg-destructive/10 px-4 py-2 text-sm text-destructive"
		>
			<span>{messages[error]}</span>
			<Button type="button" variant="ghost" size="sm" onClick={onDismiss}>
				{t("common.done")}
			</Button>
		</div>
	);
}

function ConversationCard({
	conversation,
	active,
	onClick,
}: {
	conversation: ConversationSummary;
	active: boolean;
	onClick: () => void;
}) {
	const { t } = useTranslation();
	const last = conversation.last_message;
	return (
		<button
			type="button"
			onClick={onClick}
			className={cn(
				"group grid w-full grid-cols-[2.75rem_minmax(0,1fr)] gap-3 rounded-3xl px-3 py-3 text-left transition-colors hover:bg-muted/70",
				active && "bg-primary text-primary-foreground hover:bg-primary/90",
			)}
		>
			<div
				className={cn(
					"grid size-11 place-items-center rounded-2xl bg-muted text-muted-foreground transition-colors",
					active && "bg-primary-foreground/15 text-primary-foreground",
				)}
			>
				<MessageCircle className="size-5" />
			</div>
			<div className="min-w-0">
				<div className="flex min-w-0 items-start justify-between gap-2">
					<div className="min-w-0">
						<p className="truncate text-base font-semibold leading-5">
							{conversation.phone_number}
						</p>
						<p
							className={cn(
								"mt-0.5 text-xs text-muted-foreground",
								active && "text-primary-foreground/75",
							)}
						>
							{t("messages.conversationList.messages", {
								count: conversation.total_count,
							})}
						</p>
					</div>
					<div className="flex shrink-0 flex-col items-end gap-1">
						<span
							className={cn(
								"text-xs text-muted-foreground",
								active && "text-primary-foreground/75",
							)}
						>
							{active
								? formatAbsoluteLocalTime(messageDisplayTimestamp(last))
								: formatRelativeTime(messageDisplayTimestamp(last))}
						</span>
						{conversation.unread_count > 0 && (
							<Badge
								className={cn(
									"h-5 min-w-5 px-1.5",
									active && "bg-primary-foreground text-primary",
								)}
							>
								{conversation.unread_count}
							</Badge>
						)}
					</div>
				</div>
				<div className="mt-2 flex items-center gap-2">
					<DirectionPill message={last} active={active} />
					<p
						className={cn(
							"min-w-0 flex-1 truncate text-sm text-muted-foreground",
							active && "text-primary-foreground/80",
							conversation.unread_count > 0 && "font-medium text-foreground",
							active &&
								conversation.unread_count > 0 &&
								"text-primary-foreground",
						)}
					>
						{last.body}
					</p>
				</div>
			</div>
		</button>
	);
}

function DirectionPill({
	message,
	active,
}: {
	message: Message;
	active: boolean;
}) {
	const { t } = useTranslation();
	const text =
		message.direction === "outbound"
			? t("messages.direction.sent")
			: t("messages.direction.inbox");
	return (
		<span
			className={cn(
				"shrink-0 rounded-full bg-muted px-2 py-0.5 text-[0.68rem] font-medium text-muted-foreground",
				active && "bg-primary-foreground/15 text-primary-foreground/80",
				message.status === "failed" && "bg-destructive/10 text-destructive",
			)}
		>
			{message.status === "failed" ? t("messages.direction.failed") : text}
		</span>
	);
}

function ThreadPanel({
	conversation,
	messages,
	hasOlderMessages,
	loadingOlderMessages,
	isComposingNew,
	phoneNumber,
	setPhoneNumber,
	body,
	setBody,
	sending,
	sendFailed,
	selectionMode,
	setSelectionMode,
	selectedIds,
	onToggleSelect,
	onLoadOlderMessages,
	onBack,
	onSend,
	onMarkConversationRead,
	onMarkSelectedRead,
	onMarkSelectedUnread,
	onDeleteSelected,
}: {
	conversation: ConversationSummary | null;
	messages: Message[];
	hasOlderMessages: boolean;
	loadingOlderMessages: boolean;
	isComposingNew: boolean;
	phoneNumber: string;
	setPhoneNumber: (value: string) => void;
	body: string;
	setBody: (value: string) => void;
	sending: boolean;
	sendFailed: boolean;
	selectionMode: boolean;
	setSelectionMode: (value: boolean) => void;
	selectedIds: Set<number>;
	onToggleSelect: (id: number) => void;
	onLoadOlderMessages: () => Promise<void>;
	onBack: () => void;
	onSend: () => void;
	onMarkConversationRead: () => void;
	onMarkSelectedRead: () => void;
	onMarkSelectedUnread: () => void;
	onDeleteSelected: () => void;
}) {
	const { t } = useTranslation();
	const threadScrollRef = useRef<HTMLDivElement>(null);
	const loadingOlderRef = useRef(false);
	const scrolledConversationRef = useRef<string | null>(null);
	const activeConversationRef = useRef<string | null>(null);
	activeConversationRef.current = conversation?.phone_number ?? null;
	const title = isComposingNew
		? t("messages.thread.newMessage")
		: (conversation?.phone_number ?? t("messages.thread.selectConversation"));
	const subtitle = isComposingNew
		? t("messages.thread.newMessageSubtitle")
		: conversation
			? t("messages.conversationList.messages", {
					count: conversation.total_count,
				})
			: t("messages.thread.selectConversationSubtitle");
	const selectedCount = selectedIds.size;

	useEffect(() => {
		const phone = conversation?.phone_number;
		if (!phone) {
			scrolledConversationRef.current = null;
			return;
		}
		if (messages.length === 0) return;
		if (scrolledConversationRef.current === phone) return;
		scrolledConversationRef.current = phone;
		requestAnimationFrame(() => {
			const element = threadScrollRef.current;
			if (element) element.scrollTop = element.scrollHeight;
		});
	}, [conversation?.phone_number, messages.length]);

	const loadOlderPreservingScroll = useCallback(async () => {
		if (loadingOlderRef.current || !hasOlderMessages) return;
		const conversationAtStart = activeConversationRef.current;
		const element = threadScrollRef.current;
		const previousScrollHeight = element?.scrollHeight ?? 0;
		const previousScrollTop = element?.scrollTop ?? 0;

		loadingOlderRef.current = true;
		try {
			await onLoadOlderMessages();
			if (activeConversationRef.current !== conversationAtStart) return;
			requestAnimationFrame(() => {
				if (activeConversationRef.current !== conversationAtStart) return;
				const currentElement = threadScrollRef.current;
				if (!currentElement) return;
				currentElement.scrollTop =
					previousScrollTop +
					(currentElement.scrollHeight - previousScrollHeight);
			});
		} finally {
			loadingOlderRef.current = false;
		}
	}, [hasOlderMessages, onLoadOlderMessages]);

	const handleThreadScroll = useCallback(
		(event: React.UIEvent<HTMLDivElement>) => {
			const element = event.currentTarget;
			if (
				element.scrollHeight > element.clientHeight &&
				element.scrollTop <= 64
			) {
				void loadOlderPreservingScroll();
			}
		},
		[loadOlderPreservingScroll],
	);

	return (
		<div className="flex h-full min-h-0 flex-col">
			<header className="flex shrink-0 items-center justify-between gap-3 border-b bg-background/95 px-3 py-3 backdrop-blur md:rounded-t-[min(var(--radius-4xl),28px)] md:px-5">
				<div className="flex min-w-0 items-center gap-2">
					<Button
						type="button"
						size="icon"
						variant="ghost"
						className="md:hidden"
						aria-label={t("messages.aria.backConversations")}
						onClick={onBack}
					>
						<ChevronLeft />
					</Button>
					<div className="grid size-10 shrink-0 place-items-center rounded-2xl bg-muted text-muted-foreground">
						<MessageCircle className="size-5" />
					</div>
					<div className="min-w-0">
						<h2 className="truncate text-base font-semibold">{title}</h2>
						<p className="truncate text-xs text-muted-foreground">{subtitle}</p>
					</div>
				</div>
				<div className="flex items-center gap-2">
					{conversation && (
						<Button
							type="button"
							variant="outline"
							size="icon"
							aria-label={t("messages.aria.markConversationRead")}
							onClick={onMarkConversationRead}
						>
							<CheckCheck />
						</Button>
					)}
					{conversation && (
						<ThreadActionsDropdown
							selectionMode={selectionMode}
							setSelectionMode={setSelectionMode}
							selectedCount={selectedCount}
							onMarkSelectedRead={onMarkSelectedRead}
							onMarkSelectedUnread={onMarkSelectedUnread}
							onDeleteSelected={onDeleteSelected}
						/>
					)}
				</div>
			</header>

			{conversation || isComposingNew ? (
				<>
					<div
						ref={threadScrollRef}
						role="log"
						aria-label={t("messages.aria.messageTimeline")}
						onScroll={handleThreadScroll}
						className="min-h-0 flex-1 overflow-y-auto px-3 py-4 md:px-6"
					>
						{isComposingNew ? (
							<NewMessageRecipient
								phoneNumber={phoneNumber}
								setPhoneNumber={setPhoneNumber}
							/>
						) : (
							<>
								{hasOlderMessages && (
									<div className="flex justify-center pb-3">
										<Button
											type="button"
											variant="ghost"
											size="sm"
											disabled={loadingOlderMessages}
											onClick={() => void loadOlderPreservingScroll()}
										>
											{loadingOlderMessages && (
												<LoaderCircle className="animate-spin" />
											)}
											{loadingOlderMessages
												? t("messages.thread.loadingOlder")
												: t("messages.thread.loadOlder")}
										</Button>
									</div>
								)}
								<MessageThread
									messages={messages}
									selectionMode={selectionMode}
									selectedIds={selectedIds}
									onToggleSelect={onToggleSelect}
								/>
							</>
						)}
					</div>
					<MessageComposer
						body={body}
						setBody={setBody}
						onSend={onSend}
						sending={sending}
						sendFailed={sendFailed}
						disabled={isComposingNew ? !phoneNumber.trim() : !conversation}
					/>
				</>
			) : (
				<div className="grid flex-1 place-items-center px-8 text-center">
					<div className="max-w-72 space-y-3">
						<div className="mx-auto grid size-14 place-items-center rounded-3xl bg-muted text-muted-foreground">
							<MessageCircle className="size-6" />
						</div>
						<div>
							<p className="font-medium">
								{t("messages.thread.noThreadSelected")}
							</p>
							<p className="mt-1 text-sm text-muted-foreground">
								{t("messages.thread.noThreadDescription")}
							</p>
						</div>
					</div>
				</div>
			)}
		</div>
	);
}

function ThreadActionsDropdown({
	selectionMode,
	setSelectionMode,
	selectedCount,
	onMarkSelectedRead,
	onMarkSelectedUnread,
	onDeleteSelected,
}: {
	selectionMode: boolean;
	setSelectionMode: (value: boolean) => void;
	selectedCount: number;
	onMarkSelectedRead: () => void;
	onMarkSelectedUnread: () => void;
	onDeleteSelected: () => void;
}) {
	const { t } = useTranslation();
	return (
		<DropdownMenu>
			<DropdownMenuTrigger
				render={
					<Button
						type="button"
						variant="outline"
						size="icon"
						aria-label={t("messages.aria.conversationActions")}
					/>
				}
			>
				<MoreHorizontal />
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" sideOffset={8} className="w-60">
				<DropdownMenuGroup>
					<DropdownMenuLabel>
						{t("messages.actions.conversationActions")}
					</DropdownMenuLabel>
					<DropdownMenuSeparator />
					<DropdownMenuItem onClick={() => setSelectionMode(!selectionMode)}>
						<CheckCheck />
						{selectionMode
							? t("messages.actions.stopSelecting")
							: t("messages.actions.selectMessages")}
					</DropdownMenuItem>
					<DropdownMenuItem
						disabled={selectedCount === 0}
						onClick={onMarkSelectedRead}
					>
						<CheckCheck />
						{t("messages.actions.markRead", { count: selectedCount })}
					</DropdownMenuItem>
					<DropdownMenuItem
						disabled={selectedCount === 0}
						onClick={onMarkSelectedUnread}
					>
						<MessageCircle />
						{t("messages.actions.markUnread", { count: selectedCount })}
					</DropdownMenuItem>
					<DropdownMenuSeparator />
					<DropdownMenuItem
						variant="destructive"
						disabled={selectedCount === 0}
						onClick={onDeleteSelected}
					>
						<Trash2 />
						{t("messages.actions.deleteSelected")}
					</DropdownMenuItem>
				</DropdownMenuGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function NewMessageRecipient({
	phoneNumber,
	setPhoneNumber,
}: {
	phoneNumber: string;
	setPhoneNumber: (value: string) => void;
}) {
	const { t } = useTranslation();
	return (
		<div className="mx-auto grid max-w-xl gap-2">
			<span
				className="text-xs font-medium text-muted-foreground"
				id="sms-to-label"
			>
				{t("messages.thread.recipientLabel")}
			</span>
			<Input
				aria-labelledby="sms-to-label"
				value={phoneNumber}
				onChange={(event) => setPhoneNumber(event.target.value)}
				placeholder={t("messages.thread.recipientPlaceholder")}
				className="h-11 bg-background"
			/>
		</div>
	);
}

function MessageThread({
	messages,
	selectionMode,
	selectedIds,
	onToggleSelect,
}: {
	messages: Message[];
	selectionMode: boolean;
	selectedIds: Set<number>;
	onToggleSelect: (id: number) => void;
}) {
	const { t } = useTranslation();
	if (messages.length === 0) {
		return (
			<div className="grid h-full place-items-center text-center">
				<div className="max-w-64 space-y-2">
					<p className="font-medium">
						{t("messages.conversationList.noMatching")}
					</p>
					<p className="text-sm text-muted-foreground">
						{t("messages.conversationList.noMatchingDescription")}
					</p>
				</div>
			</div>
		);
	}

	let lastDay = "";
	return (
		<div className="mx-auto flex max-w-3xl flex-col gap-2">
			{messages.map((message) => {
				const day = formatRelativeDay(messageDisplayTimestamp(message));
				const showDay = day !== lastDay;
				lastDay = day;
				return (
					<div key={message.id} className="space-y-2">
						{showDay && (
							<div className="flex justify-center py-2">
								<span className="rounded-full bg-background px-3 py-1 text-xs font-medium text-muted-foreground shadow-sm ring-1 ring-border">
									{day}
								</span>
							</div>
						)}
						<MessageBubble
							message={message}
							selectionMode={selectionMode}
							selected={selectedIds.has(message.id)}
							onToggle={() => onToggleSelect(message.id)}
						/>
					</div>
				);
			})}
		</div>
	);
}

function MessageBubble({
	message,
	selectionMode,
	selected,
	onToggle,
}: {
	message: Message;
	selectionMode: boolean;
	selected: boolean;
	onToggle: () => void;
}) {
	const outbound = message.direction === "outbound";
	return (
		<div
			className={cn(
				"flex items-end gap-2",
				outbound ? "justify-end" : "justify-start",
			)}
		>
			{selectionMode && !outbound && (
				<Checkbox checked={selected} onCheckedChange={onToggle} />
			)}
			<div
				className={cn(
					"max-w-[82%] rounded-[1.35rem] px-4 py-2.5 text-sm leading-relaxed shadow-sm ring-1",
					outbound
						? "rounded-br-md bg-primary text-primary-foreground ring-primary/10"
						: "rounded-bl-md bg-background text-foreground ring-border",
					selected && "ring-3 ring-ring/40",
				)}
			>
				<p className="whitespace-pre-wrap break-words">{message.body}</p>
				<div
					className={cn(
						"mt-1 flex flex-wrap items-center gap-1.5 text-[0.68rem]",
						outbound ? "text-primary-foreground/70" : "text-muted-foreground",
					)}
				>
					<span>{formatRelativeTime(messageDisplayTimestamp(message))}</span>
					<span>{message.status}</span>
					{message.error && (
						<span
							className={
								outbound ? "text-primary-foreground" : "text-destructive"
							}
						>
							{message.error}
						</span>
					)}
				</div>
			</div>
			{selectionMode && outbound && (
				<Checkbox checked={selected} onCheckedChange={onToggle} />
			)}
		</div>
	);
}

function MessageComposer({
	body,
	setBody,
	onSend,
	sending,
	sendFailed,
	disabled,
}: {
	body: string;
	setBody: (value: string) => void;
	onSend: () => void;
	sending: boolean;
	sendFailed: boolean;
	disabled: boolean;
}) {
	const { t } = useTranslation();
	return (
		<div className="shrink-0 border-t bg-background/95 px-3 py-3 backdrop-blur md:rounded-b-[min(var(--radius-4xl),28px)] md:px-5">
			<div className="mx-auto flex max-w-3xl items-end gap-2">
				<Textarea
					value={body}
					onChange={(event) => setBody(event.target.value)}
					placeholder={t("messages.thread.composerPlaceholder")}
					className="max-h-32 min-h-10 flex-1 bg-muted px-4 py-2.5"
					onKeyDown={(event) => {
						if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
							onSend();
						}
					}}
				/>
				<Button
					type="button"
					size="icon-lg"
					aria-label={t("messages.aria.sendMessage")}
					disabled={disabled || sending || !body.trim()}
					onClick={onSend}
				>
					<Send />
				</Button>
			</div>
			{sendFailed && (
				<p
					role="alert"
					className="mx-auto mt-2 max-w-3xl text-sm text-destructive"
				>
					{t("messages.thread.sendFailed")}
				</p>
			)}
		</div>
	);
}

function parseTime(value: string) {
	const normalized = value.replace(/([+-]\d{2})$/, "$1:00");
	const time = dayjs(normalized);
	return time.isValid() ? time : null;
}

function formatRelativeTime(value: string) {
	return parseTime(value)?.fromNow() ?? value;
}

function formatRelativeDay(value: string) {
	const time = parseTime(value);
	if (!time) return value;
	const today = dayjs().startOf("day");
	const day = time.startOf("day");
	const dayDiff = today.diff(day, "day");
	if (dayDiff === 0) return "Today";
	if (dayDiff === 1) return "Yesterday";
	if (dayDiff > 1) return `${dayDiff} days ago`;
	return day.fromNow();
}

function formatAbsoluteLocalTime(value: string) {
	const time = parseTime(value);
	if (!time) return value;
	return new Intl.DateTimeFormat(undefined, {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(time.toDate());
}
