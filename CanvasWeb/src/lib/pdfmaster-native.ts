import axios from "axios";

declare global {
    interface Window {
        webkit?: { messageHandlers?: { pdfmasterCanvas?: { postMessage: (body: unknown) => Promise<any> } } };
    }
}

export const nativeCanvas = () => location.protocol === "pdfmaster-canvas:";
export function nativeCall(body: unknown) {
    const bridge = window.webkit?.messageHandlers?.pdfmasterCanvas;
    if (!bridge) return Promise.reject(new Error("请在 PDF Master 内打开此画布。"));
    return bridge.postMessage(body);
}

const encode = async (blob: Blob) => {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let text = "";
    for (let i = 0; i < bytes.length; i += 8192) text += String.fromCharCode(...bytes.subarray(i, i + 8192));
    return btoa(text);
};

/** Native fetch adapter: never exposes credentials to JavaScript, exports, or IndexedDB. */
export function installNativeCanvas() {
    if (!nativeCanvas()) return;
    const originalFetch = window.fetch.bind(window);
    window.fetch = async (input, init) => {
        const source = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        const url = new URL(source, location.href);
        if (url.origin === location.origin && url.pathname.startsWith("/api/")) {
            if (init?.signal?.aborted) throw new DOMException("已取消", "AbortError");
            const request = new Request(input, init);
            const reply = await nativeCall({ operation: "request", path: url.pathname, method: request.method,
                contentType: request.headers.get("Content-Type") || "application/json",
                body: request.method === "GET" ? "" : await encode(await request.blob()) });
            if (init?.signal?.aborted) throw new DOMException("本机已停止等待，远端任务仍可能计费", "AbortError");
            const bytes = Uint8Array.from(atob(reply.body || ""), c => c.charCodeAt(0));
            return new Response(bytes, { status: reply.status || 200, headers: { "Content-Type": reply.contentType || "application/json" } });
        }
        // Bundled resources, data and blobs only. No analytics or hidden remote runtime.
        if (url.protocol === "pdfmaster-canvas:" || url.protocol === "data:" || url.protocol === "blob:") return originalFetch(input, init);
        throw new Error("内置版不连接外部网站服务。模型请求请使用 DMX；外部素材请先下载后导入。");
    };
    axios.defaults.adapter = "fetch";
}
