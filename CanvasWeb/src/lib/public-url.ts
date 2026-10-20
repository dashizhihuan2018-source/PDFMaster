/**
 * 子路径挂载支持。
 *
 * 以 `/` 为根部署或挂载到子路径时，静态资源由 Vite 的 `base` 负责，运行期接口与 public 资源
 * 必须走同一前缀，否则会打到宿主站点的根路径上。
 *
 * 这里统一从 `import.meta.env.BASE_URL` 取值，保证「同一份源码、两处部署」都能正常工作。
 */
const rawBase = (() => {
    const value = (import.meta as unknown as { env?: { BASE_URL?: string } }).env?.BASE_URL;
    return typeof value === "string" && value ? value : "/";
})();

/** 以 `/` 结尾的公开路径前缀。 */
export const PUBLIC_BASE = rawBase.endsWith("/") ? rawBase : `${rawBase}/`;

/** 路由 basename（无结尾斜杠）。 */
export const ROUTER_BASENAME = PUBLIC_BASE.replace(/\/+$/u, "") || "/";

/** 把 `api/auth/verify`、`/icons/x.svg` 之类的路径解析为当前部署前缀下的绝对路径。 */
export function publicUrl(path: string) {
    return `${PUBLIC_BASE}${String(path || "").replace(/^\/+/u, "")}`;
}
