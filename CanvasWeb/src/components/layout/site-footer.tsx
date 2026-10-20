import { cn } from "@/lib/utils";

/**
 * 统一的版权底栏。
 *
 * 版权主体固定为 `Nankong`，文案与 maip.top 其它产品保持一致：
 * `Copyright © 2026 Nankong. 保留所有权利 ｜ 浙ICP备2026046340号`。
 * 备案号必须链接到工信部备案系统，并在新窗口打开。
 */
export const COPYRIGHT_NOTICE = "Copyright © 2026 Nankong. 保留所有权利";
export const ICP_NOTICE = "浙ICP备2026046340号";
export const ICP_HREF = "https://beian.miit.gov.cn/";

export default function SiteFooter({ className }: { className?: string }) {
    return (
        <footer
            data-site-footer="true"
            className={cn(
                "shrink-0 border-t border-stone-200 bg-background px-4 py-1.5 dark:border-stone-800",
                className,
            )}
        >
            <p className="!my-0 flex flex-wrap items-center justify-center gap-x-1.5 gap-y-0.5 text-center text-[11px] leading-4 text-stone-500 dark:text-stone-400">
                <span>{COPYRIGHT_NOTICE}</span>
                <span aria-hidden="true" className="text-stone-300 dark:text-stone-600">
                    ｜
                </span>
                <a
                    className="transition-colors hover:text-stone-900 dark:hover:text-stone-100"
                    href={ICP_HREF}
                    rel="noreferrer"
                    target="_blank"
                >
                    {ICP_NOTICE}
                </a>
            </p>
        </footer>
    );
}
