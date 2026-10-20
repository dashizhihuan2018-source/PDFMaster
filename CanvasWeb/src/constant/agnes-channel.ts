import type { ChannelModel, ModelChannel } from "@/stores/use-config-store";
import { publicUrl } from "@/lib/public-url";

/**
 * Agnes 预设渠道（与 DMX 并列）。
 *
 * 浏览器不持有 Agnes Key：渠道指向同源的 `/api/agnes/v1`（子路径挂载时自动带上部署前缀），
 * 由运行时（runtime/agnes-bridge.mjs）在服务端注入凭证，因此这里的 apiKey 留空也不会阻断生成。
 */
export const AGNES_CHANNEL_ID = "agnes";
export const AGNES_CHANNEL_NAME = "Agnes";
export const AGNES_CHANNEL_BASE_URL = publicUrl("api/agnes/v1");

/**
 * 只列出 Agnes 上游已实测可用的模型（模型名必须小写，写错大小写会报 model_not_found）：
 * 文本 agnes-3.0-flash（支持 SSE 流式与图像输入）、图像 agnes-image-2.1-flash、
 * 视频 agnes-video-v2.0（异步任务，首帧图生视频 ti2vid 已实测）。
 * Agnes 不支持音频合成，本渠道不列 audio 模型；默认 audio 模型仍落 DMX。
 */
export const AGNES_CHANNEL_MODELS: ChannelModel[] = [
    { name: "agnes-image-2.1-flash", capability: "image" },
    { name: "agnes-video-v2.0", capability: "video" },
    { name: "agnes-3.0-flash", capability: "text" },
];

export const AGNES_DEFAULT_IMAGE_MODEL = "agnes-image-2.1-flash";
export const AGNES_DEFAULT_VIDEO_MODEL = "agnes-video-v2.0";
export const AGNES_DEFAULT_TEXT_MODEL = "agnes-3.0-flash";

/** An Agnes channel served by the local runtime bridge instead of a third-party endpoint. */
export function isAgnesChannelBaseUrl(baseUrl: string) {
    const value = (baseUrl || "").trim();
    return /(?:^|\/)api\/agnes(?:\/|$)/u.test(value) || /^https?:\/\/[^/]+\/api\/agnes(?:\/|$)/u.test(value);
}

export function createAgnesChannel(): ModelChannel {
    return {
        id: AGNES_CHANNEL_ID,
        name: AGNES_CHANNEL_NAME,
        baseUrl: AGNES_CHANNEL_BASE_URL,
        apiKey: "",
        apiFormat: "openai",
        models: AGNES_CHANNEL_MODELS.map((model) => ({ ...model })),
    };
}

export function hasAgnesChannel(channels: ModelChannel[]) {
    return channels.some((channel) => channel.id === AGNES_CHANNEL_ID || isAgnesChannelBaseUrl(channel.baseUrl));
}
