import { TanStackDevtools } from "@tanstack/react-devtools";
import {
	createRootRoute,
	Link,
	Outlet,
	useLocation,
	useNavigate,
} from "@tanstack/react-router";
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools";
import { useEffect, useState } from "react";
import { type AuthState, apiFetch } from "#/lib/api";

import "../styles.css";

export const Route = createRootRoute({
	component: RootComponent,
});

function RootComponent() {
	const location = useLocation();
	const navigate = useNavigate();
	const [auth, setAuth] = useState<AuthState | null>(null);

	useEffect(() => {
		apiFetch<AuthState>("/api/auth/me")
			.then((s) => setAuth(s))
			.catch(() => setAuth({ authenticated: false }));
	}, []);

	useEffect(() => {
		if (auth && !auth.authenticated && location.pathname !== "/login") {
			navigate({ to: "/login" });
		}
	}, [auth, location.pathname, navigate]);

	if (!auth) return null;

	if (location.pathname === "/login") {
		return (
			<>
				<Outlet />
				<TanStackDevtools
					config={{ position: "bottom-right" }}
					plugins={[
						{
							name: "TanStack Router",
							render: <TanStackRouterDevtoolsPanel />,
						},
					]}
				/>
			</>
		);
	}

	return (
		<div className="flex h-screen flex-col">
			<header className="flex items-center gap-4 border-b px-6 py-3">
				<h1 className="text-lg font-semibold">SMS Relayed</h1>
				<nav className="flex gap-4">
					<Link to="/" className="text-sm hover:underline">
						SMS
					</Link>
					<Link to="/config" className="text-sm hover:underline">
						Config
					</Link>
				</nav>
			</header>
			<main className="flex-1 overflow-auto p-6">
				<Outlet />
			</main>
			<TanStackDevtools
				config={{ position: "bottom-right" }}
				plugins={[
					{
						name: "TanStack Router",
						render: <TanStackRouterDevtoolsPanel />,
					},
				]}
			/>
		</div>
	);
}
