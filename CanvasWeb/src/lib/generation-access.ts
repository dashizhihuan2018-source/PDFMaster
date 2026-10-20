import i18n from "@/i18n";
import { canUseGeneration } from "@/services/api/auth";
import { useAuthStore } from "@/stores/use-auth-store";

/**
 * 生成调用的前端闸门。
 *
 * 访客可以浏览全部页面，但不能调用生成接口。服务端 `/api/dmx/*` 会返回 403，
 * 这里提前拦一次，让生图 / 视频 / 画布复用同一句可读的提示，而不是各自抛上游错误。
 */
export function assertCanGenerate() {
    if (canUseGeneration(useAuthStore.getState().user)) return;
    throw new Error(i18n.t("guest.generationDisabled"));
}

/** 当前身份是否可以生成。 */
export function useCanGenerate() {
    return useAuthStore((state) => state.user?.role === "admin");
}
