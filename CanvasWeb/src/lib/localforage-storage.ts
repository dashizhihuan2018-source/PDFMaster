import type { StateStorage } from "zustand/middleware";

import { scopedKey, scopedStore } from "@/lib/data-scope";

// 画布 / 资产 / 插件列表都落在这个对象存储里，按身份作用域分开（管理员沿用历史键名）。
const store = scopedStore("app_state");

export const localForageStorage: StateStorage = {
    getItem: async (name) => {
        if (typeof window === "undefined") return null;
        try {
            return (await store.getItem<string>(name)) || null;
        } catch {
            return window.localStorage.getItem(scopedKey(name));
        }
    },
    setItem: async (name, value) => {
        if (typeof window === "undefined") return;
        try {
            await store.setItem(name, value);
        } catch {
            window.localStorage.setItem(scopedKey(name), value);
        }
    },
    removeItem: async (name) => {
        if (typeof window === "undefined") return;
        try {
            await store.removeItem(name);
        } catch {
            window.localStorage.removeItem(scopedKey(name));
        }
    },
};
