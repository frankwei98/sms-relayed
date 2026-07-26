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
import { AuthContext } from "#/lib/auth";

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
			<AuthContext.Provider value={{ auth, setAuth }}>
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
			</AuthContext.Provider>
		);
	}

	const isWorkspace = ["/", "/forwarding", "/config"].includes(
		location.pathname,
	);
	const mainClassName = isWorkspace
		? "min-h-0 flex-1 overflow-hidden p-0 md:p-4"
		: "flex-1 overflow-auto p-4 md:p-6";

	return (
		<AuthContext.Provider value={{ auth, setAuth }}>
			<div className="flex h-dvh flex-col">
				<header className="flex shrink-0 items-center gap-3 border-b px-3 py-2 md:px-6 md:py-3">
					<h1 className="shrink-0 text-base font-semibold md:text-lg">
						SMS Relayed
					</h1>
					<nav
						className="flex min-w-0 flex-1 gap-1 overflow-x-auto"
						aria-label="Primary"
					>
						<Link
							to="/"
							activeOptions={{ exact: true }}
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							SMS
						</Link>
						<Link
							to="/modem"
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							Modem
						</Link>
						<Link
							to="/forwarding"
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							Forwarding
						</Link>
						<Link
							to="/config"
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							Config
						</Link>
					</nav>
				</header>
				<main className={mainClassName}>
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
		</AuthContext.Provider>
	);
}
