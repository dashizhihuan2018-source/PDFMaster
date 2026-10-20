import { publicUrl } from "@/lib/public-url";
import { nativeCanvas } from "@/lib/pdfmaster-native";

export type AuthUser = {
    username: string;
    role: "admin" | "guest";
};

/**
 * 是否可以调用生成接口。
 *
 * 浏览只要求有会话（管理员或访客都能打开全部页面），生成只认管理员；
 * 未确定身份时同样按不可生成处理。
 */
export function canUseGeneration(user: AuthUser | null | undefined) {
    return user?.role === "admin";
}

export async function verifySession(): Promise<AuthUser | null> {
    if (nativeCanvas()) return { username: "本机画布", role: "admin" };
    try {
        const response = await fetch(publicUrl("api/auth/verify"), { cache: "no-store" });
        if (!response.ok) return null;
        const user = (await response.json()) as AuthUser;
        return user.username ? user : null;
    } catch {
        return null;
    }
}

export async function login(username: string, password: string): Promise<AuthUser> {
    const response = await fetch(publicUrl("api/auth/login"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ username, password }),
    });
    const payload = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(payload.error || "invalid_credentials");
    return payload as AuthUser;
}

export async function guestLogin(): Promise<AuthUser> {
    const response = await fetch(publicUrl("api/auth/guest"), { method: "POST" });
    const payload = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(payload.error || "guest_unavailable");
    return payload as AuthUser;
}

export async function logout(): Promise<void> {
    await fetch(publicUrl("api/auth/logout"), { method: "POST", cache: "no-store" }).catch(() => undefined);
}
