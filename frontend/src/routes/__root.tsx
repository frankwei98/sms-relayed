import { TanStackDevtools } from "@tanstack/react-devtools";
import {
	createRootRoute,
	Link,
	Outlet,
	useLocation,
	useNavigate,
} from "@tanstack/react-router";
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools";
import { Globe, LogOut } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "#/components/ui/dropdown-menu";
import { AUTH_UNAUTHORIZED_EVENT, type AuthState, apiFetch } from "#/lib/api";
import { AuthContext } from "#/lib/auth";
import i18n, { type SupportedLanguage, supportedLanguages } from "#/lib/i18n";

import "../styles.css";

const languageTranslationKeys = {
	en: "language.en",
	"zh-CN": "language.zhCN",
	ja: "language.ja",
	ko: "language.ko",
	fr: "language.fr",
	es: "language.es",
} as const satisfies Record<SupportedLanguage, string>;

export const Route = createRootRoute({
	component: RootComponent,
});

function RootComponent() {
	const location = useLocation();
	const navigate = useNavigate();
	const { t } = useTranslation();
	const [auth, setAuth] = useState<AuthState | null>(null);

	useEffect(() => {
		apiFetch<AuthState>("/api/auth/me")
			.then((s) => setAuth(s))
			.catch(() => setAuth({ authenticated: false }));
	}, []);

	useEffect(() => {
		const handleUnauthorized = () => setAuth({ authenticated: false });
		window.addEventListener(AUTH_UNAUTHORIZED_EVENT, handleUnauthorized);
		return () =>
			window.removeEventListener(AUTH_UNAUTHORIZED_EVENT, handleUnauthorized);
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

	if (!auth.authenticated) return null;

	async function logout() {
		try {
			await apiFetch("/api/auth/logout", { method: "POST" });
		} catch {
			// Local logout must still hide authenticated data if the server is unreachable.
		} finally {
			setAuth({ authenticated: false });
			navigate({ to: "/login" });
		}
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
						aria-label={t("header.ariaPrimary")}
					>
						<Link
							to="/"
							activeOptions={{ exact: true }}
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							{t("nav.sms")}
						</Link>
						<Link
							to="/modem"
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							{t("nav.modem")}
						</Link>
						<Link
							to="/favorites"
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							{t("nav.favorites")}
						</Link>
						<Link
							to="/forwarding"
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							{t("nav.forwarding")}
						</Link>
						<Link
							to="/config"
							className="shrink-0 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
							activeProps={{ className: "bg-accent text-accent-foreground" }}
						>
							{t("nav.config")}
						</Link>
					</nav>
					<div className="flex shrink-0 items-center gap-1">
						<LanguageSwitcher />
						<Button
							type="button"
							variant="ghost"
							size="sm"
							aria-label={t("header.logout")}
							onClick={() => void logout()}
						>
							<LogOut />
							<span className="hidden sm:inline">{t("header.logout")}</span>
						</Button>
					</div>
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

function LanguageSwitcher() {
	const { t } = useTranslation();
	const currentLanguage = i18n.language as SupportedLanguage;

	function changeLanguage(lang: SupportedLanguage) {
		void i18n.changeLanguage(lang);
	}

	return (
		<DropdownMenu>
			<DropdownMenuTrigger
				render={
					<Button
						type="button"
						variant="ghost"
						size="icon"
						aria-label={t("language.label")}
					/>
				}
			>
				<Globe className="size-4" />
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" sideOffset={8}>
				<DropdownMenuGroup>
					<DropdownMenuLabel>{t("language.label")}</DropdownMenuLabel>
					<DropdownMenuSeparator />
					{supportedLanguages.map((lang) => (
						<DropdownMenuItem
							key={lang}
							disabled={lang === currentLanguage}
							onClick={() => changeLanguage(lang)}
						>
							{t(languageTranslationKeys[lang])}
						</DropdownMenuItem>
					))}
				</DropdownMenuGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
