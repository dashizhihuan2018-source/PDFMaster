import type { CSSProperties } from "react";
import { Tooltip } from "antd";
import { BookOpen, Keyboard, LogIn, LogOut, Puzzle, Settings2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { AnimatedThemeToggler } from "@/components/ui/animated-theme-toggler";
import { GitHubLink } from "@/components/layout/github-link";
import { VersionReleaseModal } from "@/components/layout/version-release-modal";
import { DOCS_URL } from "@/constant/env";
import { changeAppLocale, type AppLocale } from "@/i18n";
import { cn } from "@/lib/utils";
import { canvasThemes } from "@/lib/canvas-theme";
import { useConfigStore } from "@/stores/use-config-store";
import { logout } from "@/services/api/auth";
import { applyDataScope } from "@/lib/data-scope";
import { publicUrl } from "@/lib/public-url";
import { useThemeStore } from "@/stores/use-theme-store";
import { useAuthStore } from "@/stores/use-auth-store";
import { nativeCall, nativeCanvas } from "@/lib/pdfmaster-native";

type UserStatusActionsProps = {
    showConfig?: boolean;
    allowExternal?: boolean;
    variant?: "default" | "canvas";
    onOpenShortcuts?: () => void;
    onOpenPlugins?: () => void;
};

export function UserStatusActions({ showConfig = true, allowExternal = true, variant = "default", onOpenShortcuts, onOpenPlugins }: UserStatusActionsProps) {
    const { i18n, t } = useTranslation();
    const theme = useThemeStore((state) => state.theme);
    const setTheme = useThemeStore((state) => state.setTheme);
    const openConfigDialog = useConfigStore((state) => state.openConfigDialog);
    // 访客（含未登录）看到的是登录入口，管理员看到的是退出登录。
    const isAdmin = useAuthStore((state) => state.user?.role === "admin");
    if (nativeCanvas()) allowExternal = false;
    const canvasTheme = canvasThemes[theme];
    const naturalIconClass = "inline-flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-stone-600 transition-colors hover:bg-black/5 hover:text-stone-950 dark:text-stone-300 dark:hover:bg-white/10 dark:hover:text-white [&_svg]:size-4";
    const iconStyle: CSSProperties | undefined = variant === "canvas" ? { color: canvasTheme.node.text } : undefined;
    const versionStyle = iconStyle;
    const gitHubClassName = "size-7 text-base";
    const gitHubStyle = iconStyle;
    const locale = i18n.resolvedLanguage as AppLocale;
    const nextLocale = locale === "zh-CN" ? "en-US" : "zh-CN";
    const languageLabel = t("topNav.switchLanguage", { language: t(nextLocale === "zh-CN" ? "locale.zhCN" : "locale.enUS") });

    return (
        <div className="inline-flex shrink-0 items-center gap-1">
            {onOpenPlugins ? (
                <button type="button" className={naturalIconClass} style={iconStyle} onClick={onOpenPlugins} aria-label={t("topNav.plugins")} title={t("topNav.plugins")}>
                    <Puzzle className="size-4" />
                </button>
            ) : null}
            {allowExternal ? (
                <a href={DOCS_URL} target="_blank" rel="noopener noreferrer" className={naturalIconClass} style={iconStyle} aria-label={t("topNav.docs")} title={t("topNav.docs")}>
                    <BookOpen className="size-4" />
                </a>
            ) : null}
            {showConfig ? (
                <button type="button" className={naturalIconClass} style={iconStyle} onClick={() => openConfigDialog(false)} aria-label={t("navigation.config")} title={t("navigation.config")}>
                    <Settings2 className="size-4" />
                </button>
            ) : null}
            <Tooltip title={languageLabel} mouseEnterDelay={0.2}>
                <button type="button" className={`${naturalIconClass} text-[11px] font-semibold tracking-tight`} style={iconStyle} onClick={() => void changeAppLocale(nextLocale)} aria-label={languageLabel}>
                    {locale === "zh-CN" ? "中" : "EN"}
                </button>
            </Tooltip>
            <AnimatedThemeToggler theme={theme} onThemeChange={setTheme} className={naturalIconClass} style={iconStyle} aria-label={t(theme === "dark" ? "topNav.lightTheme" : "topNav.darkTheme")} title={t(theme === "dark" ? "topNav.lightTheme" : "topNav.darkTheme")} />
            {allowExternal ? <VersionReleaseModal style={versionStyle} /> : null}
            {allowExternal ? <GitHubLink className={cn("bg-transparent hover:bg-transparent dark:hover:bg-transparent", gitHubClassName)} style={gitHubStyle} /> : null}
            {onOpenShortcuts ? (
                <button type="button" className={naturalIconClass} style={iconStyle} onClick={onOpenShortcuts} aria-label={t("topNav.shortcuts")} title={t("topNav.shortcuts")}>
                    <Keyboard className="size-4" />
                </button>
            ) : null}
            {nativeCanvas() ? (
                <button type="button" className={naturalIconClass} style={iconStyle} onClick={() => void nativeCall({ operation: "settings" })} title="DMX Key 设置（本机钥匙串）" aria-label="DMX Key 设置"><Settings2 className="size-4" /></button>
            ) : isAdmin ? (
                <button
                    type="button"
                    className={naturalIconClass}
                    style={iconStyle}
                    onClick={() =>
                        void logout()
                            // 退出时先清空当前身份的内存数据，避免下一位访客在跳转前看到管理员的画布和资产。
                            .then(() => applyDataScope("pending"))
                            .then(() => window.location.assign(publicUrl("/")))
                    }
                    aria-label={t("auth.logout")}
                    title={t("auth.logout")}
                >
                    <LogOut className="size-4" />
                </button>
            ) : (
                // 访客点「登录」时必须先结束访客会话：带着会话直接跳 /login 会被服务端 302 回首页，
                // 访客就永远看不到登录表单、也无法切回管理员。
                <button
                    type="button"
                    className={naturalIconClass}
                    style={iconStyle}
                    onClick={() =>
                        void logout()
                            .then(() => applyDataScope("pending"))
                            .then(() => window.location.assign(publicUrl("/login")))
                    }
                    aria-label={t("auth.login")}
                    title={t("auth.login")}
                >
                    <LogIn className="size-4" />
                </button>
            )}
        </div>
    );
}
