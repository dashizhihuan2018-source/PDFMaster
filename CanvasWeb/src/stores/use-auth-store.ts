import { create } from "zustand";

import type { AuthUser } from "@/services/api/auth";

type AuthStore = {
    user: AuthUser | null | undefined;
    setUser: (user: AuthUser | null) => void;
    clearUser: () => void;
};

export const useAuthStore = create<AuthStore>()((set) => ({
    user: undefined,
    setUser: (user) => set({ user }),
    clearUser: () => set({ user: null }),
}));
