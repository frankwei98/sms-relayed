import { AUTH_UNAUTHORIZED_EVENT, type AuthState, apiFetch } from "./api";

export function subscribeEvents(
	handlers: Record<string, (payload: unknown) => void>,
) {
	const source = new EventSource("/api/events", { withCredentials: true });
	for (const [name, handler] of Object.entries(handlers)) {
		source.addEventListener(name, (event) => {
			handler(JSON.parse((event as MessageEvent).data));
		});
	}
	source.addEventListener("error", () => {
		void apiFetch<AuthState>("/api/auth/me")
			.then((auth) => {
				if (!auth.authenticated) {
					window.dispatchEvent(new Event(AUTH_UNAUTHORIZED_EVENT));
				}
			})
			.catch(() => {});
	});
	return () => source.close();
}
