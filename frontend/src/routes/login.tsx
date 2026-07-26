import { createFileRoute } from "@tanstack/react-router";
import {
	LOGIN_NOTICES,
	LoginForm,
	type LoginNotice,
} from "#/components/login-form";

type LoginSearch = {
	notice?: LoginNotice;
};

function isLoginNotice(value: unknown): value is LoginNotice {
	return typeof value === "string" && value in LOGIN_NOTICES;
}

export const Route = createFileRoute("/login")({
	validateSearch: (search: Record<string, unknown>): LoginSearch =>
		isLoginNotice(search.notice) ? { notice: search.notice } : {},
	component: LoginPage,
});

function LoginPage() {
	const { notice } = Route.useSearch();
	return <LoginForm notice={notice} />;
}
