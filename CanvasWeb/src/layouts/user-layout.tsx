import type { ReactNode } from "react";

import { AgentPanel } from "@/components/agent/agent-panel";
import { AppTopNav } from "@/components/layout/app-top-nav";
import SiteFooter from "@/components/layout/site-footer";
import { VisitorNotice } from "@/components/layout/visitor-notice";
import { useAuth } from "@/hooks/use-auth";

export default function UserLayout({ children }: { children: ReactNode }) {
    const user = useAuth();
    // 访客会话同样能打开全部页面；没有会话时 useAuth 会把访问者送回登录页。
    const isVisitor = user?.role === "guest";

    return (
        <div className="flex h-dvh overflow-hidden bg-background text-foreground">
            <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
                <AppTopNav user={user} />
                {isVisitor ? <VisitorNotice /> : null}
                <div className="min-h-0 flex-1 overflow-hidden">
                    {user ? (
                        children
                    ) : (
                        <div className="flex h-full items-center justify-center">
                            <span className="size-6 animate-spin rounded-full border-2 border-stone-300 border-t-stone-900 dark:border-stone-700 dark:border-t-stone-100" aria-label="Loading" />
                        </div>
                    )}
                </div>
                <SiteFooter />
            </div>
            {/* Desktop build does not ship the separate server-side Agent runtime. */}
        </div>
    );
}
