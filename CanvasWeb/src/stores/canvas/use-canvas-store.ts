import { create } from "zustand";
import { persist, type PersistStorage, type StorageValue } from "zustand/middleware";

import { nanoid } from "nanoid";
import i18n from "@/i18n";
import { currentDataScope, writeScopedItem } from "@/lib/data-scope";
import { localForageStorage } from "@/lib/localforage-storage";
import type { CanvasBackgroundMode } from "@/lib/canvas-theme";
import type { CanvasAssistantSession, CanvasConnection, CanvasNodeData, ViewportTransform } from "@/types/canvas";

export type CanvasProject = {
    id: string;
    title: string;
    createdAt: string;
    updatedAt: string;
    nodes: CanvasNodeData[];
    connections: CanvasConnection[];
    chatSessions: CanvasAssistantSession[];
    activeChatId: string | null;
    backgroundMode: CanvasBackgroundMode;
    showImageInfo: boolean;
    viewport: ViewportTransform;
};

export type CanvasDeletedProject = {
    id: string;
    deletedAt: string;
};

type CanvasStore = {
    hydrated: boolean;
    projects: CanvasProject[];
    deletedProjects: CanvasDeletedProject[];
    createProject: (title?: string) => string;
    importProject: (project: Partial<CanvasProject>) => string;
    openProject: (id: string) => CanvasProject | null;
    renameProject: (id: string, title: string) => void;
    deleteProjects: (ids: string[]) => void;
    replaceProjects: (projects: CanvasProject[], deletedProjects?: CanvasDeletedProject[]) => void;
    updateProject: (id: string, patch: Partial<Pick<CanvasProject, "nodes" | "connections" | "chatSessions" | "activeChatId" | "backgroundMode" | "showImageInfo" | "viewport">>) => void;
};

const initialViewport: ViewportTransform = { x: 0, y: 0, k: 1 };
const CANVAS_STORE_KEY = "infinite-canvas:canvas_store";
type PersistedCanvasState = Pick<CanvasStore, "projects" | "deletedProjects">;
let saveTimer: ReturnType<typeof setTimeout> | null = null;
let queuedPersistState: PersistedCanvasState | null = null;

const canvasStorage: PersistStorage<CanvasStore> = {
    getItem: async (name) => {
        const value = await localForageStorage.getItem(name);
        if (!value) return null;
        const parsed = JSON.parse(value) as StorageValue<CanvasStore>;
        queuedPersistState = parsed.state as PersistedCanvasState;
        return parsed;
    },
    setItem: (name, value) => {
        const nextState = value.state as PersistedCanvasState;
        if (queuedPersistState && queuedPersistState.projects === nextState.projects && queuedPersistState.deletedProjects === nextState.deletedProjects) return;
        queuedPersistState = nextState;
        if (saveTimer) clearTimeout(saveTimer);
        // 落盘被推迟 400ms，期间可能已经切换身份：作用域在这里固定，避免把上一个身份的画布写进新身份的库。
        const scope = currentDataScope();
        saveTimer = setTimeout(() => {
            saveTimer = null;
            void writeScopedItem("app_state", scope, name, JSON.stringify(value));
        }, 400);
    },
    removeItem: (name) => localForageStorage.removeItem(name),
};

export const useCanvasStore = create<CanvasStore>()(
    persist(
        (set, get) => ({
            hydrated: false,
            projects: [],
            deletedProjects: [],
            createProject: (title = i18n.t("canvas.project.untitled")) => {
                const now = new Date().toISOString();
                const id = nanoid();
                const project: CanvasProject = {
                    id,
                    title,
                    createdAt: now,
                    updatedAt: now,
                    nodes: [],
                    connections: [],
                    chatSessions: [],
                    activeChatId: null,
                    backgroundMode: "lines",
                    showImageInfo: false,
                    viewport: initialViewport,
                };
                set((state) => ({ projects: [project, ...state.projects] }));
                return id;
            },
            importProject: (source) => {
                const now = new Date().toISOString();
                const project: CanvasProject = {
                    id: nanoid(),
                    title: source.title || i18n.t("canvas.project.imported"),
                    createdAt: source.createdAt || now,
                    updatedAt: now,
                    nodes: source.nodes || [],
                    connections: source.connections || [],
                    chatSessions: source.chatSessions || [],
                    activeChatId: source.activeChatId || null,
                    backgroundMode: source.backgroundMode || "lines",
                    showImageInfo: source.showImageInfo || false,
                    viewport: source.viewport || initialViewport,
                };
                set((state) => ({ projects: [project, ...state.projects] }));
                return project.id;
            },
            openProject: (id) => {
                return get().projects.find((item) => item.id === id) || null;
            },
            renameProject: (id, title) =>
                set((state) => ({
                    projects: state.projects.map((project) => (project.id === id ? { ...project, title: title.trim() || project.title, updatedAt: new Date().toISOString() } : project)),
                })),
            deleteProjects: (ids) =>
                set((state) => {
                    const now = new Date().toISOString();
                    const removing = new Set(ids);
                    const projects = state.projects.filter((project) => !removing.has(project.id));
                    const deletedProjects = [...state.deletedProjects.filter((item) => !removing.has(item.id)), ...ids.map((id) => ({ id, deletedAt: now }))];
                    return { projects, deletedProjects };
                }),
            replaceProjects: (projects, deletedProjects = []) => set({ projects, deletedProjects }),
            updateProject: (id, patch) =>
                set((state) => ({
                    projects: state.projects.map((project) => (project.id === id ? { ...project, ...patch, updatedAt: new Date().toISOString() } : project)),
                })),
        }),
        {
            name: CANVAS_STORE_KEY,
            storage: canvasStorage,
            partialize: (state) =>
                ({
                    projects: state.projects,
                    deletedProjects: state.deletedProjects,
                }) as StorageValue<CanvasStore>["state"],
            // 切换身份后新作用域可能是空的：此时必须落回空画布，
            // 否则内存里还留着上一个身份（例如管理员）的画布。
            merge: (persisted, current) => {
                const state = (persisted || {}) as Partial<PersistedCanvasState>;
                return {
                    ...current,
                    projects: Array.isArray(state.projects) ? state.projects : [],
                    deletedProjects: Array.isArray(state.deletedProjects) ? state.deletedProjects : [],
                };
            },
            onRehydrateStorage: () => () => {
                useCanvasStore.setState({ hydrated: true });
            },
        },
    ),
);
