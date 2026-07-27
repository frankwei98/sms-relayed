import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import { type AuthState, apiFetch } from "#/lib/api";
import { useAuth } from "#/lib/auth";

export const LOGIN_NOTICES = {
	config_saved_restart_scheduled:
		"Configuration saved and restart scheduled. Sign in with the new password after the service returns.",
} as const;

export type LoginNotice = keyof typeof LOGIN_NOTICES;

type LoginSearch = {
	notice?: LoginNotice;
};

function isLoginNotice(value: unknown): value is LoginNotice {
	return typeof value === "string" && Object.hasOwn(LOGIN_NOTICES, value);
}

export const Route = createFileRoute("/login")({
	validateSearch: (search: Record<string, unknown>): LoginSearch =>
		isLoginNotice(search.notice) ? { notice: search.notice } : {},
	component: LoginPage,
});

function LoginPage() {
	const navigate = useNavigate();
	const { notice } = Route.useSearch();
	const { setAuth } = useAuth();
	const [password, setPassword] = useState("");
	const [error, setError] = useState("");
	const { t } = useTranslation();

	async function handleSubmit(e: React.FormEvent) {
		e.preventDefault();
		setError("");
		try {
			const res = await apiFetch<AuthState>("/api/auth/login", {
				method: "POST",
				body: JSON.stringify({ password }),
			});
			if (res.authenticated) {
				setAuth(res);
				navigate({ to: "/" });
			}
		} catch (err: unknown) {
			setError((err as Error).message ?? t("login.loginFailed"));
		}
	}

	return (
		<div className="flex min-h-dvh items-center justify-center p-4">
			<form
				onSubmit={handleSubmit}
				className="mx-auto w-full max-w-sm space-y-4 rounded-lg border p-6"
			>
				<h1 className="text-xl font-semibold">{t("login.title")}</h1>
				{notice ? (
					<p className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm">
						{t("login.notice.configSavedRestart")}
					</p>
				) : null}
				{error ? <p className="text-sm text-destructive">{error}</p> : null}
				<Input
					type="password"
					placeholder={t("login.password")}
					value={password}
					onChange={(e) => setPassword(e.target.value)}
				/>
				<Button type="submit" className="w-full">
					{t("login.login")}
				</Button>
			</form>
		</div>
	);
}
