import { createRouter, RouterProvider } from "@tanstack/react-router";
import ReactDOM from "react-dom/client";
import "./lib/i18n";
import {
	initMonitoring,
	loadMonitoringPreference,
	Sentry,
} from "./lib/monitoring";
import { routeTree } from "./routeTree.gen";

const router = createRouter({
	routeTree,
	defaultPreload: "intent",
	scrollRestoration: true,
});

declare module "@tanstack/react-router" {
	interface Register {
		router: typeof router;
	}
}

const rootElement = document.getElementById("app") as HTMLElement;

async function start(): Promise<void> {
	initMonitoring(await loadMonitoringPreference());
	if (rootElement.innerHTML) return;

	const root = ReactDOM.createRoot(rootElement, {
		onUncaughtError: Sentry.reactErrorHandler(),
		onRecoverableError: Sentry.reactErrorHandler(),
	});
	root.render(
		<Sentry.ErrorBoundary
			fallback={
				<div role="alert" className="p-6 text-sm">
					The dashboard encountered an unexpected error. Refresh to try again.
				</div>
			}
		>
			<RouterProvider router={router} />
		</Sentry.ErrorBoundary>,
	);
}

void start();
