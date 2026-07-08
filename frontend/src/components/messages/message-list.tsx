import { Checkbox } from "#/components/ui/checkbox";
import type { Message } from "#/lib/api";

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
						<Checkbox disabled />
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
							<Checkbox
								checked={selectedIds.has(msg.id)}
								onCheckedChange={() => onToggle(msg.id)}
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
