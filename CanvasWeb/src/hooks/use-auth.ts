import { useEffect } from "react";
import { useNavigate } from "react-router-dom";

import { applyDataScope, scopeForUser } from "@/lib/data-scope";
import { verifySession, type AuthUser } from "@/services/api/auth";
import { useAuthStore } from "@/stores/use-auth-store";

/**
 * 当前访问身份。
 *
 * 站点要求先建立会话：没有会话时回登录页。在登录页点「以访客身份进入」会拿到
 * 访客会话，之后全站页面都能打开，只是生成接口仍然只认管理员。
 *
 * 会话确定后先把本机数据切到对应作用域（管理员 / 访客各一份），再交给界面渲染，
 * 避免访客看到上一个身份留在浏览器里的画布和资产。
 * 返回值在会话校验完成前是 `undefined`。
 */
export function useAuth() {
    const user = useAuthStore((state) => state.user);
    const setUser = useAuthStore((state) => state.setUser);
    const navigate = useNavigate();

    useEffect(() => {
        if (user !== undefined) return;
        let cancelled = false;
        void verifySession()
            .then(async (verified) => {
                if (cancelled) return;
                await applyDataScope(scopeForUser(verified));
                if (cancelled) return;
                setUser(verified);
                if (!verified) navigate("/login", { replace: true });
            })
            .catch(() => {
                if (!cancelled) setUser(null);
            });
        return () => {
            cancelled = true;
        };
    }, [navigate, setUser, user]);

    return user;
}

/** 仅用于确实需要管理员身份的场景；不满足时返回 null，由调用方决定如何提示。 */
export function useRequireRole(role: AuthUser["role"]): AuthUser | null {
    const user = useAuth();
    return user?.role === role ? user : null;
}
