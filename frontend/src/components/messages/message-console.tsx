import { useCallback, useEffect, useState } from "react";
import { Button } from "#/components/ui/button";
import { apiFetch, type ConversationSummary, type Message } from "#/lib/api";
import { subscribeEvents } from "#/lib/events";
import {
	MessageFilters,
	MessageList,
	SendMessageForm,
} from "./message-filters";

export function MessageConsole() {
	const [conversations, setConversations] = useState<ConversationSummary[]>([]);
	const [messages, setMessages] = useState<Message[]>([]);
	const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
	const [q, setQ] = useState("");
	const [phoneFilter, setPhoneFilter] = useState("");
	const [direction, setDirection] = useState("");
	const [statusFilter, setStatusFilter] = useState("");
	const [unreadOnly, setUnreadOnly] = useState(false);
	const [phoneNumber, setPhoneNumber] = useState("");
	const [body, setBody] = useState("");
	const [sending, setSending] = useState(false);

	const buildParams = useCallback(() => {
		const p = new URLSearchParams();
		if (q) p.set("q", q);
		if (phoneFilter) p.set("phone_number", phoneFilter);
		if (direction) p.set("direction", direction);
		if (statusFilter) p.set("status", statusFilter);
		if (unreadOnly) p.set("unread", "true");
		return p;
	}, [q, phoneFilter, direction, statusFilter, unreadOnly]);

	const loadMessages = useCallback(async () => {
		const p = buildParams();
		const data = await apiFetch<Message[]>(`/api/messages?${p.toString()}`);
		setMessages(data);
	}, [buildParams]);

	const loadConversations = useCallback(async () => {
		const data = await apiFetch<ConversationSummary[]>("/api/conversations");
		setConversations(data);
	}, []);

	useEffect(() => {
		loadMessages();
		loadConversations();
	}, [loadMessages, loadConversations]);

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

	function toggleSelect(id: number) {
		setSelectedIds((prev) => {
			const next = new Set(prev);
			if (next.has(id)) next.delete(id);
			else next.add(id);
			return next;
		});
	}

	async function handleSend() {
		if (!phoneNumber.trim() || !body.trim()) return;
		setSending(true);
		try {
			await apiFetch<Message>("/api/messages/send", {
				method: "POST",
				body: JSON.stringify({ phone_number: phoneNumber, body }),
			});
			setBody("");
			await loadMessages();
			await loadConversations();
		} catch (err) {
			console.error(err);
		} finally {
			setSending(false);
		}
	}

	async function handleMarkRead(id: number) {
		await apiFetch(`/api/messages/${id}/read`, { method: "POST" });
		await loadMessages();
		await loadConversations();
	}

	async function handleMarkUnread(id: number) {
		await apiFetch(`/api/messages/${id}/unread`, { method: "POST" });
		await loadMessages();
		await loadConversations();
	}

	async function handleDeleteMany() {
		if (selectedIds.size === 0) return;
		const ids = Array.from(selectedIds);
		await apiFetch("/api/messages/delete", {
			method: "POST",
			body: JSON.stringify({ ids }),
		});
		setSelectedIds(new Set());
		await loadMessages();
		await loadConversations();
	}

	return (
		<div className="space-y-4">
			<div className="flex items-center gap-2">
				<h2 className="text-lg font-semibold">Conversations</h2>
			</div>
			<div className="flex flex-wrap gap-2">
				{conversations.map((c) => (
					<button
						key={c.phone_number}
						type="button"
						onClick={() => setPhoneFilter(c.phone_number)}
						className="rounded border px-3 py-1 text-sm hover:bg-muted"
					>
						{c.phone_number}
						{c.unread_count > 0 && (
							<span className="ml-1 rounded bg-red-500 px-1 text-xs text-white">
								{c.unread_count}
							</span>
						)}
					</button>
				))}
			</div>

			<MessageFilters
				q={q}
				setQ={setQ}
				phoneFilter={phoneFilter}
				setPhoneFilter={setPhoneFilter}
				direction={direction}
				setDirection={setDirection}
				statusFilter={statusFilter}
				setStatusFilter={setStatusFilter}
				unreadOnly={unreadOnly}
				setUnreadOnly={setUnreadOnly}
				buildParams={buildParams}
			/>

			<div className="flex items-center gap-2">
				<Button
					variant="outline"
					size="sm"
					onClick={async () => {
						if (selectedIds.size === 1) {
							await handleMarkRead(Array.from(selectedIds)[0]);
						}
					}}
					disabled={selectedIds.size !== 1}
				>
					Mark Read
				</Button>
				<Button
					variant="outline"
					size="sm"
					onClick={async () => {
						if (selectedIds.size === 1) {
							await handleMarkUnread(Array.from(selectedIds)[0]);
						}
					}}
					disabled={selectedIds.size !== 1}
				>
					Mark Unread
				</Button>
				<Button
					variant="outline"
					size="sm"
					onClick={handleDeleteMany}
					disabled={selectedIds.size === 0}
				>
					Delete Selected ({selectedIds.size})
				</Button>
			</div>

			<MessageList
				messages={messages}
				selectedIds={selectedIds}
				onToggle={toggleSelect}
			/>

			<div className="mt-4 space-y-2">
				<h3 className="text-sm font-semibold">Send SMS</h3>
				<SendMessageForm
					phoneNumber={phoneNumber}
					setPhoneNumber={setPhoneNumber}
					body={body}
					setBody={setBody}
					onSend={handleSend}
					sending={sending}
				/>
			</div>
		</div>
	);
}
