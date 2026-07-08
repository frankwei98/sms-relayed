import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";

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
