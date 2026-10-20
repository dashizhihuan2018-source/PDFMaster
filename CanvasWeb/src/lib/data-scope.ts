import localforage from "localforage";
import type { StateStorage } from "zustand/middleware";

/**
 * 本机数据作用域：同一个浏览器里，管理员和访客不能共用一份数据。
 *
 * 画布、资产、生成历史、渠道配置都保存在浏览器本机（IndexedDB + localStorage），键名原本不分身份。
 * 于是管理员退出登录、再从访客入口进来时，访客会直接看到管理员的画布和资产。
 * 这里给所有「属于某个身份」的存储加作用域后缀：
 *
 * - `admin`：沿用改造前的键名，历史数据零迁移、零丢失；
 * - `guest`：键名统一追加 `__guest`，访客拿到的是自己的空库；
 * - `pending`：身份还没校验出来时的临时作用域，避免登录页渲染阶段先读到上一个身份的数据。
 *
 * 身份确定或退出后必须调用 `applyDataScope()`：光换后缀不够，zustand 的内存态还是上一个身份的，
 * 需要把内存态清空再按新作用域重新水合。
 *
 * 不参与作用域的只有两类：主题等纯外观偏好，以及提示词注册表等网络缓存 —— 它们不含用户内容。
 */
export type DataScope = "pending" | "admin" | "guest";

let currentScope: DataScope = "pending";

export function currentDataScope(): DataScope {
    return currentScope;
}

/** 会话身份 → 数据作用域：管理员一份、访客一份、未确定时用 pending。 */
export function scopeForUser(user: { role: "admin" | "guest" } | null | undefined): DataScope {
    if (!user) return "pending";
    return user.role === "admin" ? "admin" : "guest";
}

/** 作用域后缀；admin 保持历史键名。 */
export function scopeSuffix(scope: DataScope = currentScope) {
    return scope === "admin" ? "" : `__${scope}`;
}

/** `app_state` → `app_state__guest`。 */
export function scopedName(base: string, scope: DataScope = currentScope) {
    return `${base}${scopeSuffix(scope)}`;
}

/** localStorage 键名同样按身份隔离。 */
export function scopedKey(base: string, scope: DataScope = currentScope) {
    return scopedName(base, scope);
}

const instances = new Map<string, LocalForage>();

function instanceFor(base: string, scope: DataScope) {
    const storeName = scopedName(base, scope);
    let created = instances.get(storeName);
    if (!created) {
        created = localforage.createInstance({ name: "infinite-canvas", storeName });
        instances.set(storeName, created);
    }
    return created;
}

/** 只暴露项目里实际用到的方法，调用时再按当前作用域取实例。 */
export type ScopedStore = {
    getItem<T>(key: string): Promise<T | null>;
    setItem<T>(key: string, value: T): Promise<T>;
    removeItem(key: string): Promise<void>;
    clear(): Promise<void>;
    keys(): Promise<string[]>;
    length(): Promise<number>;
    key(index: number): Promise<string | null>;
    iterate<T, U>(iteratee: (value: T, key: string, iterationNumber: number) => U): Promise<U | undefined>;
    dropInstance(): Promise<void>;
    /** 当前解析到的对象存储名，用于诊断和测试。 */
    name(): string;
};

/**
 * 拿到一个「跟随当前身份」的对象存储句柄。
 *
 * 句柄本身在模块加载时创建，但每次调用才解析真实实例，所以身份切换后无需重建调用方。
 */
export function scopedStore(base: string, fixedScope?: DataScope): ScopedStore {
    // 不传 fixedScope 时按「调用时」的当前身份解析：模块级句柄在身份切换后必须自动跟着走。
    const resolve = () => fixedScope ?? currentScope;
    return {
        getItem: <T>(key: string) => instanceFor(base, resolve()).getItem<T>(key),
        setItem: <T>(key: string, value: T) => instanceFor(base, resolve()).setItem<T>(key, value),
        removeItem: (key: string) => instanceFor(base, resolve()).removeItem(key),
        clear: () => instanceFor(base, resolve()).clear(),
        keys: () => instanceFor(base, resolve()).keys(),
        length: () => instanceFor(base, resolve()).length(),
        key: (index: number) => instanceFor(base, resolve()).key(index),
        iterate: <T, U>(iteratee: (value: T, key: string, iterationNumber: number) => U) => instanceFor(base, resolve()).iterate<T, U>(iteratee),
        dropInstance: () => instanceFor(base, resolve()).dropInstance(),
        name: () => scopedName(base, resolve()),
    };
}

/**
 * 把某个键写进指定作用域。
 *
 * 给「延迟落盘」的调用点用：写入被排到几百毫秒后时，期间可能已经切换身份，
 * 这时必须写回它原本所属的作用域，否则会把上一个身份的数据写进新身份的库。
 */
export async function writeScopedItem(base: string, scope: DataScope, key: string, value: string) {
    await scopedStore(base, scope).setItem(key, value);
}

/** localStorage 版的按身份取键，给 zustand 的 JSON storage 用。 */
export function scopedLocalStorage(): StateStorage {
    return {
        getItem: (name) => (typeof window === "undefined" ? null : window.localStorage.getItem(scopedKey(name))),
        setItem: (name, value) => {
            if (typeof window !== "undefined") window.localStorage.setItem(scopedKey(name), value);
        },
        removeItem: (name) => {
            if (typeof window !== "undefined") window.localStorage.removeItem(scopedKey(name));
        },
    };
}

/**
 * 按身份重新装载所有含用户内容的 store。
 *
 * 动态 import 有两个原因：避免与 store 形成循环依赖；切换身份发生在登录 / 校验完成之后，
 * 代码此时早已在包内，不需要额外的网络请求。
 *
 * 这里只重新水合，不再手工清空内存：含用户内容的 store 都带 `merge`，
 * 新作用域没有数据时会落回空值，有数据时才装载它自己的数据。
 */
async function userDataStores() {
    const [canvas, asset, config, prompts, plugin] = await Promise.all([
        import("@/stores/canvas/use-canvas-store"),
        import("@/stores/use-asset-store"),
        import("@/stores/use-config-store"),
        import("@/stores/use-prompt-source-store"),
        import("@/stores/canvas/use-plugin-store"),
    ]);
    return [
        async () => {
            await canvas.useCanvasStore.persist.rehydrate();
        },
        async () => {
            await asset.useAssetStore.persist.rehydrate();
        },
        async () => {
            await plugin.usePluginStore.persist.rehydrate();
        },
        // 渠道配置、提示词源自带 merge：新作用域为空时落回默认值，访客不会带上管理员的 Key。
        async () => {
            await config.useConfigStore.persist.rehydrate();
        },
        async () => {
            await prompts.usePromptSourceStore.persist.rehydrate();
        },
    ];
}

/**
 * 切换本机数据作用域并把 store 重新水合到新身份。
 *
 * - 首次确定身份（pending → admin / guest）：清空上一个身份的残留，装载本身份的数据；
 * - 退出登录（→ pending）：只清空内存态，下次登录再装载。
 */
export async function applyDataScope(scope: DataScope) {
    if (scope === currentScope && scope !== "pending") return;
    currentScope = scope;

    const stores = await userDataStores();
    await Promise.all(stores.map((rehydrate) => rehydrate()));
}
