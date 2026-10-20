import { Info } from "lucide-react";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";

/**
 * 访客模式提示条。
 *
 * 访客可以浏览全部页面，只是不能调用生成接口；这里把边界说清楚，
 * 避免访客在生图 / 视频工作台或画布上反复点一个必然失败的按钮。
 */
export function VisitorNotice() {
    const { t } = useTranslation();

    return (
        <div className="flex shrink-0 flex-wrap items-center justify-center gap-x-3 gap-y-1 border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-900 dark:border-amber-900/40 dark:bg-amber-950/30 dark:text-amber-200">
            <span className="inline-flex items-center gap-1.5">
                <Info className="size-3.5 shrink-0" />
                {t("guest.notice")}
            </span>
            <Link to="/login" className="shrink-0 font-medium underline decoration-amber-400 underline-offset-2 transition hover:decoration-amber-700">
                {t("guest.login")}
            </Link>
        </div>
    );
}
