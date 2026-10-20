import React from "react";

import { emitCanvasEvent, onCanvasEvent } from "@/lib/canvas/canvas-event-bus";
import type { CanvasPluginApp, CanvasPluginHost } from "@/types/canvas-plugin";

// Remote plugins obtain the host React instance through this runtime to avoid multiple React copies.
export type PluginRuntime = CanvasPluginApp & {
    React: typeof React;
    jsx: typeof React.createElement;
    Fragment: typeof React.Fragment;
    injectCSS: (css: string, key?: string) => () => void;
    // 画板宿主编排句柄：在画布页挂载后可用，插件因此能在没有节点被渲染时也读取/修改画板。
    host: CanvasPluginHost | null;
};

let runtime: PluginRuntime | null = null;
let canvasHost: CanvasPluginHost | null = null;

/** 画布页在挂载期间登记自己的宿主编排句柄；卸载后清空，避免插件拿到过期引用。 */
export function setCanvasPluginHost(host: CanvasPluginHost | null) {
    canvasHost = host;
}

export function getCanvasPluginHost(): CanvasPluginHost | null {
    return canvasHost;
}

// Inject plugin styles, replacing the previous style with the same key, and return a removal function.
function injectCSS(css: string, key?: string) {
    const id = key ? `canvas-plugin-style-${key}` : undefined;
    if (id) document.getElementById(id)?.remove();
    const style = document.createElement("style");
    if (id) style.id = id;
    style.dataset.canvasPluginStyle = "true";
    style.textContent = css;
    document.head.appendChild(style);
    return () => style.remove();
}

export function getPluginRuntime(): PluginRuntime {
    if (!runtime) {
        runtime = {
            React,
            jsx: React.createElement,
            Fragment: React.Fragment,
            injectCSS,
            version: typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "dev",
            emit: emitCanvasEvent,
            on: onCanvasEvent,
            get host() {
                return canvasHost;
            },
        };
        (window as unknown as { InfiniteCanvasRuntime?: PluginRuntime }).InfiniteCanvasRuntime = runtime;
    }
    return runtime;
}
