import dayjs from "dayjs";
import relativeTime from "dayjs/plugin/relativeTime";
import {
	Archive,
	CheckCheck,
	ChevronLeft,
	Download,
	Filter,
	Inbox,
	MessageCircle,
	MoreHorizontal,
	Plus,
	Search,
	Send,
	Trash2,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { cn } from "#/lib/utils";

dayjs.extend(relativeTime);
dayjs.locale(navigator.language.toLowerCase());

const ALL_DIRECTIONS = "all-directions";
const ALL_STATUSES = "all-statuses";

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
	const markingReadIdsRef = useRef<Set<number>>(new Set());

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

	const loadMessagesForPhone = useCallback(
		async (phone: string) => {
			const p = buildParams(phone);
			const data = await apiFetch<Message[]>(`/api/messages?${p.toString()}`);
			setMessages(data);
		},
		[buildParams],
	);

	const loadMessages = useCallback(async () => {
		if (!selectedPhone) {
			setMessages([]);
			return;
		}
		await loadMessagesForPhone(selectedPhone);
	}, [loadMessagesForPhone, selectedPhone]);

	const loadConversations = useCallback(async () => {
		const data = await apiFetch<ConversationSummary[]>("/api/conversations");
		setConversations(data);
	}, []);

	const reloadActiveViews = useCallback(async () => {
		await Promise.all([loadMessages(), loadConversations()]);
	}, [loadMessages, loadConversations]);

	useEffect(() => {
		loadConversations();
	}, [loadConversations]);

	useEffect(() => {
		loadMessages();
	}, [loadMessages]);

	useEffect(() => {
		const unsub = subscribeEvents({
			"message.created": () => {
				loadMessages();
				loadConversations();
			},
			"message.updated": () => {
				loadMessages();
				loadConversations();
			},
			"message.deleted": () => {
				loadMessages();
				loadConversations();
			},
			"message.read_state_changed": () => {
				loadMessages();
				loadConversations();
			},
		});
		return unsub;
	}, [loadMessages, loadConversations]);

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
		const unreadVisibleIds = messages
			.filter(
				(message) =>
					message.phone_number === selectedPhone &&
					message.direction === "inbound" &&
					message.read_at === null &&
					!markingReadIdsRef.current.has(message.id),
			)
			.map((message) => message.id);
		if (unreadVisibleIds.length === 0) return;

		for (const id of unreadVisibleIds) {
			markingReadIdsRef.current.add(id);
		}

		Promise.all(
			unreadVisibleIds.map((id) =>
				apiFetch(`/api/messages/${id}/read`, { method: "POST" }),
			),
		)
			.then(async () => {
				await reloadActiveViews();
			})
			.catch((err) => {
				console.error(err);
			})
			.finally(() => {
				for (const id of unreadVisibleIds) {
					markingReadIdsRef.current.delete(id);
				}
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

	const orderedMessages = useMemo(() => [...messages].reverse(), [messages]);
	const activePhone = isComposingNew ? phoneNumber : selectedPhone;
	const hasThreadOpen = Boolean(selectedPhone || isComposingNew);

	function selectConversation(phone: string) {
		setSelectedPhone(phone);
		setPhoneNumber(phone);
		setIsComposingNew(false);
		setSelectionMode(false);
		setSelectedIds(new Set());
	}

	function startNewMessage() {
		setSelectedPhone(null);
		setPhoneNumber("");
		setMessages([]);
		setIsComposingNew(true);
		setSelectionMode(false);
		setSelectedIds(new Set());
	}

	function closeMobileThread() {
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
		setSending(true);
		try {
			await apiFetch<Message>("/api/messages/send", {
				method: "POST",
				body: JSON.stringify({ phone_number: recipient, body }),
			});
			setBody("");
			setSelectedPhone(recipient);
			setPhoneNumber(recipient);
			setIsComposingNew(false);
			await Promise.all([loadMessagesForPhone(recipient), loadConversations()]);
		} catch (err) {
			console.error(err);
		} finally {
			setSending(false);
		}
	}

	async function handleMarkConversationRead() {
		if (!selectedPhone) return;
		try {
			await apiFetch(
				`/api/conversations/${encodeURIComponent(selectedPhone)}/read`,
				{
					method: "POST",
				},
			);
			await reloadActiveViews();
		} catch (err) {
			console.error(err);
		}
	}

	async function handleMarkSelected(read: boolean) {
		const ids = Array.from(selectedIds);
		if (ids.length === 0) return;
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
			console.error(err);
		}
	}

	async function handleDeleteSelected() {
		if (selectedIds.size === 0) return;
		try {
			await apiFetch("/api/messages/delete", {
				method: "POST",
				body: JSON.stringify({ ids: Array.from(selectedIds) }),
			});
			setSelectedIds(new Set());
			setSelectionMode(false);
			await reloadActiveViews();
		} catch (err) {
			console.error(err);
		}
	}

	function exportMessages(format: "csv" | "json") {
		const p = buildParams(selectedPhone);
		window.location.href = `/api/messages/export?format=${format}&${p.toString()}`;
	}

	return (
		<div className="flex h-full min-h-0 flex-col bg-background">
			<div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden md:grid-cols-[22rem_minmax(0,1fr)] md:gap-4">
				<section
					className={cn(
						"flex min-h-0 flex-col border-border bg-background md:flex",
						hasThreadOpen && "hidden md:flex",
					)}
				>
					<ConversationListHeader
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
						messages={orderedMessages}
						isComposingNew={isComposingNew}
						phoneNumber={phoneNumber}
						setPhoneNumber={setPhoneNumber}
						body={body}
						setBody={setBody}
						sending={sending}
						selectionMode={selectionMode}
						setSelectionMode={setSelectionMode}
						selectedIds={selectedIds}
						onToggleSelect={toggleSelect}
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
	query,
	onQueryChange,
	onNewMessage,
	filters,
}: {
	query: string;
	onQueryChange: (value: string) => void;
	onNewMessage: () => void;
	filters: ReactNode;
}) {
	return (
		<div className="shrink-0 border-b bg-background/95 px-4 py-3 backdrop-blur md:rounded-t-[min(var(--radius-4xl),28px)] md:border-x md:border-t">
			<div className="flex items-center justify-between gap-3">
				<div>
					<p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
						SMS
					</p>
					<h2 className="font-heading text-2xl font-semibold tracking-normal">
						Messages
					</h2>
				</div>
				<div className="flex items-center gap-2">
					{filters}
					<Button
						type="button"
						size="icon"
						variant="default"
						aria-label="New message"
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
					placeholder="Search messages"
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
	return (
		<Dialog>
			<DialogTrigger
				render={
					<Button
						type="button"
						size="icon"
						variant="outline"
						aria-label="Filters"
					/>
				}
			>
				<Filter />
			</DialogTrigger>
			<DialogContent className="gap-5">
				<DialogHeader>
					<DialogTitle>Message tools</DialogTitle>
					<DialogDescription>
						Filter the inbox or export the current message view.
					</DialogDescription>
				</DialogHeader>
				<div className="grid gap-4">
					<div className="grid gap-1.5">
						<span className="text-xs font-medium text-muted-foreground">
							Direction
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
								<SelectItem value={ALL_DIRECTIONS}>All directions</SelectItem>
								<SelectItem value="inbound">Inbound</SelectItem>
								<SelectItem value="outbound">Outbound</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="grid gap-1.5">
						<span className="text-xs font-medium text-muted-foreground">
							Status
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
								<SelectItem value={ALL_STATUSES}>All statuses</SelectItem>
								<SelectItem value="received">Received</SelectItem>
								<SelectItem value="sending">Sending</SelectItem>
								<SelectItem value="sent">Sent</SelectItem>
								<SelectItem value="failed">Failed</SelectItem>
							</SelectContent>
						</Select>
					</div>
					<div className="flex items-center justify-between rounded-2xl bg-muted/70 px-3 py-2">
						<span className="text-sm font-medium">Unread only</span>
						<Checkbox
							checked={unreadOnly}
							onCheckedChange={(checked) => setUnreadOnly(checked === true)}
						/>
					</div>
				</div>
				<DialogFooter className="grid grid-cols-2 sm:flex">
					<Button type="button" variant="outline" onClick={onExportCsv}>
						<Download />
						CSV
					</Button>
					<Button type="button" variant="outline" onClick={onExportJson}>
						<Archive />
						JSON
					</Button>
					<DialogClose render={<Button type="button" variant="secondary" />}>
						Done
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
}: {
	conversations: ConversationSummary[];
	selectedPhone: string | null;
	onSelect: (phone: string) => void;
}) {
	if (conversations.length === 0) {
		return (
			<div className="grid flex-1 place-items-center px-6 text-center">
				<div className="max-w-64 space-y-3">
					<div className="mx-auto grid size-12 place-items-center rounded-2xl bg-muted text-muted-foreground">
						<Inbox className="size-5" />
					</div>
					<div>
						<p className="font-medium">No conversations</p>
						<p className="mt-1 text-sm text-muted-foreground">
							Incoming and outgoing SMS threads will appear here.
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

function ConversationCard({
	conversation,
	active,
	onClick,
}: {
	conversation: ConversationSummary;
	active: boolean;
	onClick: () => void;
}) {
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
							{conversation.total_count} messages
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
								? formatAbsoluteLocalTime(last.timestamp)
								: formatRelativeTime(last.timestamp)}
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
	const text = message.direction === "outbound" ? "Sent" : "Inbox";
	return (
		<span
			className={cn(
				"shrink-0 rounded-full bg-muted px-2 py-0.5 text-[0.68rem] font-medium text-muted-foreground",
				active && "bg-primary-foreground/15 text-primary-foreground/80",
				message.status === "failed" && "bg-destructive/10 text-destructive",
			)}
		>
			{message.status === "failed" ? "Failed" : text}
		</span>
	);
}

function ThreadPanel({
	conversation,
	messages,
	isComposingNew,
	phoneNumber,
	setPhoneNumber,
	body,
	setBody,
	sending,
	selectionMode,
	setSelectionMode,
	selectedIds,
	onToggleSelect,
	onBack,
	onSend,
	onMarkConversationRead,
	onMarkSelectedRead,
	onMarkSelectedUnread,
	onDeleteSelected,
}: {
	conversation: ConversationSummary | null;
	messages: Message[];
	isComposingNew: boolean;
	phoneNumber: string;
	setPhoneNumber: (value: string) => void;
	body: string;
	setBody: (value: string) => void;
	sending: boolean;
	selectionMode: boolean;
	setSelectionMode: (value: boolean) => void;
	selectedIds: Set<number>;
	onToggleSelect: (id: number) => void;
	onBack: () => void;
	onSend: () => void;
	onMarkConversationRead: () => void;
	onMarkSelectedRead: () => void;
	onMarkSelectedUnread: () => void;
	onDeleteSelected: () => void;
}) {
	const title = isComposingNew
		? "New message"
		: (conversation?.phone_number ?? "Select a conversation");
	const subtitle = isComposingNew
		? "Choose a recipient and write an SMS"
		: conversation
			? `${conversation.total_count} messages`
			: "Pick a thread from the list";
	const selectedCount = selectedIds.size;

	return (
		<div className="flex h-full min-h-0 flex-col">
			<header className="flex shrink-0 items-center justify-between gap-3 border-b bg-background/95 px-3 py-3 backdrop-blur md:rounded-t-[min(var(--radius-4xl),28px)] md:px-5">
				<div className="flex min-w-0 items-center gap-2">
					<Button
						type="button"
						size="icon"
						variant="ghost"
						className="md:hidden"
						aria-label="Back to conversations"
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
							aria-label="Mark conversation read"
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
					<div className="min-h-0 flex-1 overflow-y-auto px-3 py-4 md:px-6">
						{isComposingNew ? (
							<NewMessageRecipient
								phoneNumber={phoneNumber}
								setPhoneNumber={setPhoneNumber}
							/>
						) : (
							<MessageThread
								messages={messages}
								selectionMode={selectionMode}
								selectedIds={selectedIds}
								onToggleSelect={onToggleSelect}
							/>
						)}
					</div>
					<MessageComposer
						body={body}
						setBody={setBody}
						onSend={onSend}
						sending={sending}
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
							<p className="font-medium">No thread selected</p>
							<p className="mt-1 text-sm text-muted-foreground">
								Choose a conversation or start a new SMS.
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
	return (
		<DropdownMenu>
			<DropdownMenuTrigger
				render={
					<Button
						type="button"
						variant="outline"
						size="icon"
						aria-label="Conversation actions"
					/>
				}
			>
				<MoreHorizontal />
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" sideOffset={8} className="w-60">
				<DropdownMenuGroup>
					<DropdownMenuLabel>Conversation actions</DropdownMenuLabel>
					<DropdownMenuSeparator />
					<DropdownMenuItem onClick={() => setSelectionMode(!selectionMode)}>
						<CheckCheck />
						{selectionMode ? "Stop selecting" : "Select messages"}
					</DropdownMenuItem>
					<DropdownMenuItem
						disabled={selectedCount === 0}
						onClick={onMarkSelectedRead}
					>
						<CheckCheck />
						Mark read ({selectedCount})
					</DropdownMenuItem>
					<DropdownMenuItem
						disabled={selectedCount === 0}
						onClick={onMarkSelectedUnread}
					>
						<MessageCircle />
						Mark unread ({selectedCount})
					</DropdownMenuItem>
					<DropdownMenuSeparator />
					<DropdownMenuItem
						variant="destructive"
						disabled={selectedCount === 0}
						onClick={onDeleteSelected}
					>
						<Trash2 />
						Delete selected
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
	return (
		<div className="mx-auto grid max-w-xl gap-2">
			<span
				className="text-xs font-medium text-muted-foreground"
				id="sms-to-label"
			>
				To
			</span>
			<Input
				aria-labelledby="sms-to-label"
				value={phoneNumber}
				onChange={(event) => setPhoneNumber(event.target.value)}
				placeholder="Phone number"
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
	if (messages.length === 0) {
		return (
			<div className="grid h-full place-items-center text-center">
				<div className="max-w-64 space-y-2">
					<p className="font-medium">No matching messages</p>
					<p className="text-sm text-muted-foreground">
						Adjust filters or wait for the next SMS event.
					</p>
				</div>
			</div>
		);
	}

	let lastDay = "";
	return (
		<div className="mx-auto flex max-w-3xl flex-col gap-2">
			{messages.map((message) => {
				const day = formatRelativeDay(message.timestamp);
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
					<span>{formatRelativeTime(message.timestamp)}</span>
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
	disabled,
}: {
	body: string;
	setBody: (value: string) => void;
	onSend: () => void;
	sending: boolean;
	disabled: boolean;
}) {
	return (
		<div className="shrink-0 border-t bg-background/95 px-3 py-3 backdrop-blur md:rounded-b-[min(var(--radius-4xl),28px)] md:px-5">
			<div className="mx-auto flex max-w-3xl items-end gap-2">
				<Textarea
					value={body}
					onChange={(event) => setBody(event.target.value)}
					placeholder="Message"
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
					aria-label="Send message"
					disabled={disabled || sending || !body.trim()}
					onClick={onSend}
				>
					<Send />
				</Button>
			</div>
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
