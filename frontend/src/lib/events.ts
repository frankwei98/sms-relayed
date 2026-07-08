export function subscribeEvents(
	handlers: Record<string, (payload: unknown) => void>,
) {
	const source = new EventSource("/api/events", { withCredentials: true });
	for (const [name, handler] of Object.entries(handlers)) {
		source.addEventListener(name, (event) => {
			handler(JSON.parse((event as MessageEvent).data));
		});
	}
	return () => source.close();
}
