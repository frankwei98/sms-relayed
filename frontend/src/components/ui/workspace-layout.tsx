import type { ReactNode } from "react";
import { cn } from "#/lib/utils";

type WorkspaceLayoutProps = {
	navigation: ReactNode;
	children: ReactNode;
	mobileNavigationOpen: boolean;
	navigationLabel: string;
	className?: string;
};

export function WorkspaceLayout({
	navigation,
	children,
	mobileNavigationOpen,
	navigationLabel,
	className,
}: WorkspaceLayoutProps) {
	return (
		<div
			className={cn(
				"h-full min-h-0 overflow-hidden bg-background md:rounded-xl md:border",
				className,
			)}
		>
			<div className="grid h-full min-h-0 md:grid-cols-[17rem_minmax(0,1fr)]">
				<aside
					aria-label={navigationLabel}
					className={cn(
						"min-h-0 flex-col bg-sidebar text-sidebar-foreground md:flex md:border-r",
						mobileNavigationOpen ? "flex" : "hidden",
					)}
				>
					{navigation}
				</aside>
				<section
					className={cn(
						"min-h-0 flex-col bg-background md:flex",
						mobileNavigationOpen ? "hidden" : "flex",
					)}
				>
					{children}
				</section>
			</div>
		</div>
	);
}
