import { useState } from "react";
import { Button, Input, App } from "antd";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { guestLogin, login } from "@/services/api/auth";
import { applyDataScope, scopeForUser } from "@/lib/data-scope";
import { publicUrl } from "@/lib/public-url";
import { useAuthStore } from "@/stores/use-auth-store";
import SiteFooter from "@/components/layout/site-footer";

export default function LoginPage() {
    const { t } = useTranslation();
    const { message } = App.useApp();
    const navigate = useNavigate();
    const setUser = useAuthStore((state) => state.setUser);
    const [username, setUsername] = useState("");
    const [password, setPassword] = useState("");
    const [loading, setLoading] = useState(false);

    async function submit(admin: boolean) {
        setLoading(true);
        try {
            const user = admin ? await login(username, password) : await guestLogin();
            // 先切到该身份的本机数据作用域，再进入应用：访客拿到的是自己的空画布 / 空资产。
            await applyDataScope(scopeForUser(user));
            setUser(user);
            navigate("/", { replace: true });
        } catch (error) {
            message.error(error instanceof Error && error.message !== "invalid_credentials" ? error.message : t("auth.invalid"));
        } finally {
            setLoading(false);
        }
    }

    return (
        <main className="flex min-h-dvh flex-col bg-background">
            <div className="flex flex-1 items-center justify-center px-6 py-12">
                <div className="w-full max-w-md rounded-2xl border border-stone-200 bg-white p-8 shadow-sm dark:border-stone-800 dark:bg-stone-900">
                    <div className="mb-8 flex flex-col items-center gap-3 text-center">
                        <img src={publicUrl("logo.svg")} alt={t("meta.title")} className="size-12 object-contain dark:invert" />
                        <div>
                            <h1 className="text-2xl font-semibold text-stone-950 dark:text-stone-100">{t("meta.title")}</h1>
                            <p className="mt-2 text-sm text-stone-500 dark:text-stone-400">{t("auth.description")}</p>
                        </div>
                    </div>
                    <form
                        className="space-y-4"
                        onSubmit={(event) => {
                            event.preventDefault();
                            void submit(true);
                        }}
                    >
                        <label className="block space-y-2">
                            <span className="text-sm font-medium text-stone-700 dark:text-stone-300">{t("auth.username")}</span>
                            <Input size="large" value={username} autoComplete="username" onChange={(event) => setUsername(event.target.value)} />
                        </label>
                        <label className="block space-y-2">
                            <span className="text-sm font-medium text-stone-700 dark:text-stone-300">{t("auth.password")}</span>
                            <Input.Password size="large" value={password} autoComplete="current-password" onChange={(event) => setPassword(event.target.value)} />
                        </label>
                        <Button type="primary" size="large" htmlType="submit" block loading={loading}>{t("auth.login")}</Button>
                    </form>
                    <Button type="default" size="large" block className="mt-3" loading={loading} onClick={() => void submit(false)}>{t("auth.guest")}</Button>
                    <section className="mt-8 border-t border-stone-200 pt-5 dark:border-stone-800">
                        <h2 className="text-sm font-semibold text-stone-950 dark:text-stone-100">{t("auth.previewTitle")}</h2>
                        <p className="mt-2 text-xs leading-5 text-stone-500 dark:text-stone-400">{t("auth.previewDescription")}</p>
                    </section>
                </div>
            </div>
            <SiteFooter />
        </main>
    );
}
