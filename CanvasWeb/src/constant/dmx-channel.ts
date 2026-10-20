import type { ChannelModel, ModelChannel } from "@/stores/use-config-store";
import { publicUrl } from "@/lib/public-url";

/**
 * DMX 预设渠道。
 *
 * 浏览器不持有 DMX Key：渠道指向同源的 `/api/dmx/v1`（子路径挂载时自动带上部署前缀），
 * 由 PDF Master 原生桥从钥匙串读取凭证，因此这里的 apiKey 留空也不会阻断生成。
 */
export const DMX_CHANNEL_ID = "dmx";
export const DMX_CHANNEL_NAME = "DMX";
export const DMX_CHANNEL_BASE_URL = publicUrl("api/dmx/v1");

/**
 * 桌面预设模型。是否可用以用户的 DMX 权限、余额与服务状态为准。
 * 视频仅提供 Seedance 2.5 文生或单首帧；本轮没有使用真实 Key 验收付费生成。
 */
export const DMX_CHANNEL_MODELS: ChannelModel[] = [
    { name: "gpt-image-2.5-sunburst", capability: "image" },
    { name: "gpt-image-2.5-flare", capability: "image" },
    { name: "gpt-image-2", capability: "image" },
    { name: "gemini-3-pro-image", capability: "image" },
    { name: "gemini-3.1-flash-image", capability: "image" },
    { name: "qwen-image-3.0-pro", capability: "image" },
    { name: "wan2.7-image-pro", capability: "image" },
    { name: "z-image-turbo", capability: "image" },
    { name: "doubao-seedance-2-5-260628", capability: "video" },
    { name: "gpt-5.6-sol", capability: "text" },
    { name: "gpt-5.5", capability: "text" },
    { name: "glm-5.3", capability: "text" },
    { name: "gpt-4o-mini-tts", capability: "audio" },
    { name: "speech-2.6-hd", capability: "audio" },
];

export const DMX_DEFAULT_IMAGE_MODEL = "gpt-image-2.5-sunburst";
export const DMX_DEFAULT_VIDEO_MODEL = "doubao-seedance-2-5-260628";
export const DMX_DEFAULT_TEXT_MODEL = "gpt-5.6-sol";
export const DMX_DEFAULT_AUDIO_MODEL = "gpt-4o-mini-tts";

/** A channel served by the local runtime bridge instead of a third-party endpoint. */
export function isManagedChannelBaseUrl(baseUrl: string) {
    const value = (baseUrl || "").trim();
    // Agnes 桥接（runtime/agnes-bridge.mjs）与 DMX 同为本地 runtime 托管渠道：
    // 匹配到这里即视为“浏览器无 key 也放行”，密钥由服务端注入。
    return (
        /(?:^|\/)api\/dmx(?:\/|$)/u.test(value) ||
        /^https?:\/\/[^/]+\/api\/dmx(?:\/|$)/u.test(value) ||
        /(?:^|\/)api\/agnes(?:\/|$)/u.test(value) ||
        /^https?:\/\/[^/]+\/api\/agnes(?:\/|$)/u.test(value)
    );
}

function isDmxChannelBaseUrl(baseUrl: string) {
    const value = (baseUrl || "").trim();
    return /(?:^|\/)api\/dmx(?:\/|$)/u.test(value) || /^https?:\/\/[^/]+\/api\/dmx(?:\/|$)/u.test(value);
}

export function createDmxChannel(): ModelChannel {
    return {
        id: DMX_CHANNEL_ID,
        name: DMX_CHANNEL_NAME,
        baseUrl: DMX_CHANNEL_BASE_URL,
        apiKey: "",
        apiFormat: "openai",
        models: DMX_CHANNEL_MODELS.map((model) => ({ ...model })),
    };
}

/** The DMX preset is always offered; detecting it by its own URL so a removed DMX channel
 * comes back on the next load regardless of which other managed channels are present. */
export function hasDmxChannel(channels: ModelChannel[]) {
    return channels.some((channel) => channel.id === DMX_CHANNEL_ID || isDmxChannelBaseUrl(channel.baseUrl));
}
