import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import type { Message } from "#/lib/api";

type FiltersProps = {
	q: string;
	setQ: (v: string) => void;
	phoneFilter: string;
	setPhoneFilter: (v: string) => void;
	direction: string;
	setDirection: (v: string) => void;
	statusFilter: string;
	setStatusFilter: (v: string) => void;
	unreadOnly: boolean;
	setUnreadOnly: (v: boolean) => void;
	buildParams: () => URLSearchParams;
};

export function MessageFilters({
	q,
	setQ,
	phoneFilter,
	setPhoneFilter,
	direction,
	setDirection,
	statusFilter,
	setStatusFilter,
	unreadOnly,
	setUnreadOnly,
	buildParams,
}: FiltersProps) {
	return (
		<div className="flex flex-wrap items-end gap-2">
			<div className="flex flex-col gap-1">
				<span className="text-xs text-muted-foreground">Search</span>
				<Input
					placeholder="Search text..."
					value={q}
					onChange={(e) => setQ(e.target.value)}
					className="h-8 w-40"
				/>
			</div>
			<div className="flex flex-col gap-1">
				<span className="text-xs text-muted-foreground">Phone</span>
				<Input
					placeholder="Number..."
					value={phoneFilter}
					onChange={(e) => setPhoneFilter(e.target.value)}
					className="h-8 w-36"
				/>
			</div>
			<select
				className="h-8 rounded border px-2 text-sm"
				value={direction}
				onChange={(e) => setDirection(e.target.value)}
			>
				<option value="">All directions</option>
				<option value="inbound">Inbound</option>
				<option value="outbound">Outbound</option>
			</select>
			<select
				className="h-8 rounded border px-2 text-sm"
				value={statusFilter}
				onChange={(e) => setStatusFilter(e.target.value)}
			>
				<option value="">All statuses</option>
				<option value="received">Received</option>
				<option value="sending">Sending</option>
				<option value="sent">Sent</option>
				<option value="failed">Failed</option>
			</select>
			<span className="flex items-center gap-1 text-sm">
				<input
					type="checkbox"
					checked={unreadOnly}
					onChange={(e) => setUnreadOnly(e.target.checked)}
				/>
				Unread only
			</span>
			<Button
				variant="outline"
				size="sm"
				onClick={() => {
					window.location.href = `/api/messages/export?format=csv&${buildParams().toString()}`;
				}}
			>
				CSV
			</Button>
			<Button
				variant="outline"
				size="sm"
				onClick={() => {
					window.location.href = `/api/messages/export?format=json&${buildParams().toString()}`;
				}}
			>
				JSON
			</Button>
		</div>
	);
}

function statusBadge(s: Message["status"]) {
	switch (s) {
		case "received":
			return <span className="text-green-600">Received</span>;
		case "sending":
			return <span className="text-yellow-600">Sending</span>;
		case "sent":
			return <span className="text-blue-600">Sent</span>;
		case "failed":
			return <span className="text-red-600">Failed</span>;
	}
}

export function MessageList({
	messages,
	selectedIds,
	onToggle,
}: {
	messages: Message[];
	selectedIds: Set<number>;
	onToggle: (id: number) => void;
}) {
	return (
		<table className="w-full text-sm">
			<thead>
				<tr className="border-b text-left">
					<th className="w-8 p-2">
						<input type="checkbox" disabled />
					</th>
					<th className="p-2">Dir</th>
					<th className="p-2">Number</th>
					<th className="p-2">Body</th>
					<th className="p-2">Time</th>
					<th className="p-2">Status</th>
				</tr>
			</thead>
			<tbody>
				{messages.map((msg) => (
					<tr key={msg.id} className="border-b hover:bg-muted/50">
						<td className="p-2">
							<input
								type="checkbox"
								checked={selectedIds.has(msg.id)}
								onChange={() => onToggle(msg.id)}
							/>
						</td>
						<td className="p-2">{msg.direction === "inbound" ? "←" : "→"}</td>
						<td className="p-2 font-mono text-xs">{msg.phone_number}</td>
						<td className="max-w-xs truncate p-2">{msg.body}</td>
						<td className="p-2 text-xs text-muted-foreground">
							{msg.timestamp}
						</td>
						<td className="p-2">{statusBadge(msg.status)}</td>
					</tr>
				))}
				{messages.length === 0 && (
					<tr>
						<td colSpan={6} className="p-4 text-center text-muted-foreground">
							No messages
						</td>
					</tr>
				)}
			</tbody>
		</table>
	);
}

export function SendMessageForm({
	phoneNumber,
	setPhoneNumber,
	body,
	setBody,
	onSend,
	sending,
}: {
	phoneNumber: string;
	setPhoneNumber: (v: string) => void;
	body: string;
	setBody: (v: string) => void;
	onSend: () => void;
	sending: boolean;
}) {
	return (
		<div className="flex flex-wrap items-end gap-2">
			<Input
				placeholder="Phone number"
				value={phoneNumber}
				onChange={(e) => setPhoneNumber(e.target.value)}
				className="h-8 w-44"
			/>
			<Input
				placeholder="Message text"
				value={body}
				onChange={(e) => setBody(e.target.value)}
				className="h-8 flex-1 min-w-40"
			/>
			<Button size="sm" onClick={onSend} disabled={sending}>
				{sending ? "Sending..." : "Send"}
			</Button>
		</div>
	);
}
