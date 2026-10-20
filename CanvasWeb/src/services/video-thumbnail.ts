// 视频缩略图：浏览器内用离屏 <video> seek 到关键帧，canvas 截一帧生成 WebP，再按视频存储 key
// 缓存在本地 IndexedDB（video_thumbnails）。与图片缩略图（image-storage.ts 的 preview 机制）同构：
// 只存本地、不写进节点数据、不参与导出与 WebDAV 同步；解析失败时静默返回，界面保留原兜底显示。
import { scopedStore } from "@/lib/data-scope";

export const VIDEO_THUMBNAIL_MAX_EDGE = 768;

const store = scopedStore("video_thumbnails");

type StoredVideoThumbnail = { version: number; createdAt: number; blob?: Blob };

const VIDEO_THUMBNAIL_VERSION = 1;
// 取 35% 处一帧，避开黑片头、又不落到结尾。
const VIDEO_FRAME_SEEK_RATIO = 0.35;
const VIDEO_SEEK_TIMEOUT_MS = 6000;
const VIDEO_MAX_EDGE = VIDEO_THUMBNAIL_MAX_EDGE;

const thumbnailUrls = new Map<string, string>();
const thumbnailListeners = new Set<() => void>();
let thumbnailRevision = 0;
let thumbnailQueue: Promise<unknown> = Promise.resolve();
const inflight = new Map<string, Promise<string | undefined>>();

export function videoThumbnailUrlFor(storageKey?: string) {
    return storageKey ? thumbnailUrls.get(storageKey) : undefined;
}

export function subscribeVideoThumbnails(listener: () => void) {
    thumbnailListeners.add(listener);
    return () => {
        thumbnailListeners.delete(listener);
    };
}

export function getVideoThumbnailRevision() {
    return thumbnailRevision;
}

// 先查内存缓存，再查本地 IndexedDB；都没有就排队生成。已生成过的不会重复生成。
// 同一 storageKey 并发调用只生成一次，避免重复 seek/截帧。
export function ensureVideoThumbnail(storageKey?: string): Promise<string | undefined> {
    if (!storageKey || !storageKey.startsWith("video:")) return Promise.resolve(undefined);
    const cached = thumbnailUrls.get(storageKey);
    if (cached) return Promise.resolve(cached);
    const pending = inflight.get(storageKey);
    if (pending) return pending;
    const job = queueVideoThumbnail(storageKey);
    inflight.set(storageKey, job);
    void job
        .then(() => {
            inflight.delete(storageKey);
        })
        .catch(() => {
            inflight.delete(storageKey);
        });
    return job;
}

function queueVideoThumbnail(storageKey: string) {
    const run = async (): Promise<string | undefined> => {
        const stored = await store.getItem<StoredVideoThumbnail>(storageKey).catch(() => null);
        if (stored?.version === VIDEO_THUMBNAIL_VERSION) {
            if (stored.blob) return cacheVideoThumbnail(storageKey, stored.blob);
            return undefined;
        }
        // 动态导入，避免模块顶层依赖链（i18n/配置 store）在测试环境里提前求值。
        const { getMediaBlob } = await import("@/services/file-storage").catch(() => ({ getMediaBlob: async () => null }));
        const original = await getMediaBlob(storageKey).catch(() => null);
        if (!original) return undefined;
        const blob = await createVideoThumbnail(original).catch(() => undefined);
        if (!blob) return undefined;
        await store.setItem<StoredVideoThumbnail>(storageKey, { version: VIDEO_THUMBNAIL_VERSION, createdAt: Date.now(), blob }).catch(() => undefined);
        return cacheVideoThumbnail(storageKey, blob);
    };
    const job = thumbnailQueue.then(run).catch(() => undefined);
    thumbnailQueue = job;
    return job;
}

// 从原始视频 blob 截一帧：createObjectURL → 离屏 <video> seek → canvas 截帧 → WebP。
async function createVideoThumbnail(original: Blob): Promise<Blob | undefined> {
    const url = URL.createObjectURL(original);
    try {
        const video = await loadVideoFrame(url);
        if (!video) return undefined;
        try {
            const width = video.videoWidth || 1280;
            const height = video.videoHeight || 720;
            const scale = Math.min(1, VIDEO_MAX_EDGE / Math.max(width, height));
            const canvas = document.createElement("canvas");
            canvas.width = Math.max(1, Math.round(width * scale));
            canvas.height = Math.max(1, Math.round(height * scale));
            const context = canvas.getContext("2d");
            if (!context) return undefined;
            context.drawImage(video, 0, 0, canvas.width, canvas.height);
            return await canvasToBlob(canvas);
        } finally {
            video.remove();
        }
    } finally {
        URL.revokeObjectURL(url);
    }
}

// 加载一帧：等到 seeked（或兜底超时）后，画面就绪就返回该 <video>，解析不了返回 undefined。
function loadVideoFrame(url: string) {
    return new Promise<HTMLVideoElement | undefined>((resolve) => {
        const video = document.createElement("video");
        let settled = false;
        const finish = (value: HTMLVideoElement | undefined) => {
            if (settled) return;
            settled = true;
            window.clearTimeout(timer);
            video.onloadeddata = null;
            video.onseeked = null;
            video.onerror = null;
            resolve(value);
        };
        const timer = window.setTimeout(() => finish(video.videoWidth && video.videoHeight ? video : undefined), VIDEO_SEEK_TIMEOUT_MS);
        video.muted = true;
        video.playsInline = true;
        video.preload = "auto";
        video.onloadeddata = () => {
            const target = Number.isFinite(video.duration) && video.duration > 0 ? video.duration * VIDEO_FRAME_SEEK_RATIO : 1;
            try {
                video.currentTime = target;
            } catch {
                // 无法 seek 时保留当前帧（通常是第 0 帧），兜底超时仍会结算。
            }
        };
        video.onseeked = () => {
            // seek 完成后画面已就绪，立即结算，不用等兜底超时。
            if (video.videoWidth && video.videoHeight) finish(video);
        };
        video.onerror = () => finish(undefined);
        video.src = url;
        document.body.appendChild(video);
    });
}

function canvasToBlob(canvas: HTMLCanvasElement) {
    return new Promise<Blob | undefined>((resolve) => canvas.toBlob((result) => resolve(result || undefined), "image/webp", 0.86));
}

function cacheVideoThumbnail(storageKey: string, blob: Blob) {
    const url = URL.createObjectURL(blob);
    thumbnailUrls.set(storageKey, url);
    thumbnailRevision += 1;
    thumbnailListeners.forEach((listener) => listener());
    return url;
}

export async function deleteVideoThumbnail(storageKey: string) {
    const url = thumbnailUrls.get(storageKey);
    if (url) URL.revokeObjectURL(url);
    thumbnailUrls.delete(storageKey);
    inflight.delete(storageKey);
    await store.removeItem(storageKey).catch(() => undefined);
}

// 清掉不再被任何资产/项目引用的视频缩略图，和媒体原件的清理规则一致。
export async function cleanupUnusedVideoThumbnails(usedData: unknown) {
    const usedKeys = collectVideoThumbnailKeys(usedData);
    const unused: string[] = [];
    await store.iterate((_value, key) => {
        if (typeof key === "string" && !usedKeys.has(key)) unused.push(key);
    });
    await Promise.all(unused.map(deleteVideoThumbnail));
}

export function collectVideoThumbnailKeys(value: unknown, keys = new Set<string>()) {
    if (!value || typeof value !== "object") return keys;
    if ("storageKey" in value && typeof value.storageKey === "string" && value.storageKey.startsWith("video:")) keys.add(value.storageKey);
    Object.values(value).forEach((item) => (Array.isArray(item) ? item.forEach((child) => collectVideoThumbnailKeys(child, keys)) : collectVideoThumbnailKeys(item, keys)));
    return keys;
}
