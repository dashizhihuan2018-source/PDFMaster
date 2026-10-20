/**
 * 宿主皮肤桥接。
 *
 * 画布被建筑 Agent 等工作区以内嵌方式打开时，宿主的界面皮肤需要同步到画布自身的明暗主题，
 * 否则会出现「深色工作区里嵌着一块浅色画布」。宿主在打开画布与切换皮肤时会 postMessage
 * 主题名；独立部署的站点收不到该消息，保持自己原有的主题设置。
 */
import { useThemeStore } from "@/stores/use-theme-store";

type HostThemeMessage = { type?: string; theme?: string };

export function installHostThemeBridge() {
    if (typeof window === "undefined") return () => undefined;
    const onMessage = (event: MessageEvent<HostThemeMessage>) => {
        if (event.origin !== window.location.origin) return;
        const payload = event.data;
        if (!payload || payload.type !== "workspace:canvas-theme") return;
        const theme = payload.theme === "dark" ? "dark" : "light";
        if (useThemeStore.getState().theme !== theme) useThemeStore.getState().setTheme(theme);
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
}
