import { Card, Tag } from "antd";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { GUEST_PREVIEWS } from "@/constant/guest-previews";

export default function GuestPreviewsPage() {
    const { t } = useTranslation();
    return (
        <main className="h-full overflow-y-auto bg-background">
            <section className="mx-auto max-w-6xl px-6 py-12">
                <h1 className="text-3xl font-semibold text-stone-950 dark:text-stone-100">{t("guestPreviews.title")}</h1>
                <p className="mt-3 max-w-3xl text-sm leading-6 text-stone-500 dark:text-stone-400">{t("guestPreviews.description")}</p>
                <div className="mt-8 grid gap-5 md:grid-cols-3">
                    {GUEST_PREVIEWS.map((item) => (
                        <Link key={item.id} to={`/guest-previews/${item.id}`} className="block h-full focus-visible:outline-none" aria-label={item.title}>
                        <Card
                            hoverable
                            cover={
                                item.mediaType === "video" && item.videoUrl ? (
                                    <video src={item.videoUrl} poster={item.coverUrl} controls preload="metadata" playsInline className="aspect-[4/3] w-full bg-stone-950 object-cover" aria-label={item.title} />
                                ) : (
                                    <img src={item.coverUrl} alt={item.title} className="aspect-[4/3] object-cover" />
                                )
                            }
                            className="h-full overflow-hidden"
                        >
                            <div className="mb-2 flex flex-wrap gap-1.5">{item.tags.map((tag) => <Tag key={tag}>{tag}</Tag>)}</div>
                            <h2 className="text-base font-semibold text-stone-950 dark:text-stone-100">{item.title}</h2>
                            <p className="mt-1 text-xs text-stone-500 dark:text-stone-400">{item.location}</p>
                            <p className="mt-3 text-sm leading-6 text-stone-600 dark:text-stone-300">{item.description}</p>
                            <p className="mt-3 text-xs font-medium text-stone-900 dark:text-stone-100">{t("guestPreviews.openDetail")} →</p>
                        </Card>
                        </Link>
                    ))}
                </div>
            </section>
        </main>
    );
}
