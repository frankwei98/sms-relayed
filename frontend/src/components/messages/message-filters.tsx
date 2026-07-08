import { Button } from "#/components/ui/button";
import { Checkbox } from "#/components/ui/checkbox";
import { Input } from "#/components/ui/input";

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
				<Checkbox
					checked={unreadOnly}
					onCheckedChange={(c) => setUnreadOnly(c === true)}
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
