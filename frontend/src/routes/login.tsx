import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import { type AuthState, apiFetch } from "#/lib/api";
import { useAuth } from "#/lib/auth";

type LoginSearch = {
	notice?: string;
};

export const Route = createFileRoute("/login")({
	validateSearch: (search: Record<string, unknown>): LoginSearch => ({
		notice: typeof search.notice === "string" ? search.notice : undefined,
	}),
	component: LoginPage,
});

function LoginPage() {
	const navigate = useNavigate();
	const { notice } = Route.useSearch();
	const { setAuth } = useAuth();
	const [password, setPassword] = useState("");
	const [error, setError] = useState("");

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
			setError((err as Error).message ?? "Login failed");
		}
	}

	return (
		<div className="flex min-h-dvh items-center justify-center p-4">
			<form
				onSubmit={handleSubmit}
				className="mx-auto w-full max-w-sm space-y-4 rounded-lg border p-6"
			>
				<h1 className="text-xl font-semibold">SMS Relayed</h1>
				{notice ? (
					<p className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm">
						{notice}
					</p>
				) : null}
				{error ? <p className="text-sm text-destructive">{error}</p> : null}
				<Input
					type="password"
					placeholder="Password"
					value={password}
					onChange={(e) => setPassword(e.target.value)}
				/>
				<Button type="submit" className="w-full">
					Login
				</Button>
			</form>
		</div>
	);
}
