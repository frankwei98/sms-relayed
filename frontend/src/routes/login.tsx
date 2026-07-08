import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import { type AuthState, apiFetch } from "#/lib/api";

export const Route = createFileRoute("/login")({
	component: LoginPage,
});

function LoginPage() {
	const navigate = useNavigate();
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
				navigate({ to: "/" });
			}
		} catch (err: unknown) {
			setError((err as Error).message ?? "Login failed");
		}
	}

	return (
		<div className="flex min-h-screen items-center justify-center">
			<form
				onSubmit={handleSubmit}
				className="mx-auto w-full max-w-sm space-y-4 rounded-lg border p-6"
			>
				<h1 className="text-xl font-semibold">SMS Relayed</h1>
				{error && <p className="text-sm text-red-500">{error}</p>}
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
