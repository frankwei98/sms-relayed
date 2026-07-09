import { createContext, useContext } from "react";
import type { AuthState } from "#/lib/api";

type AuthContextValue = {
	auth: AuthState;
	setAuth: (auth: AuthState) => void;
};

export const AuthContext = createContext<AuthContextValue | null>(null);

export function useAuth() {
	const context = useContext(AuthContext);
	if (!context) {
		throw new Error("useAuth must be used within AuthContext");
	}
	return context;
}
