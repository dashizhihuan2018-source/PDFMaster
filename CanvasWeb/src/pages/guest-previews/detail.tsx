import { ArrowLeftOutlined, PlayCircleOutlined } from "@ant-design/icons";
import { Card, Tag } from "antd";
import { useTranslation } from "react-i18next";
import { Link, useParams } from "react-router-dom";

import { GUEST_PREVIEWS } from "@/constant/guest-previews";

export default function GuestPreviewDetailPage() {
    const { t } = useTranslation();
    const { id } = useParams();
    const item = GUEST_PREVIEWS.find((entry) => entry.id === id);

    if (!item) {
        return (
            <main className="h-full overflow-y-auto bg-background">
                <section className="mx-auto max-w-3xl px-6 py-16">
                    <h1 className="text-2xl font-semibold text-stone-950 dark:text-stone-100">{t("guestPreviews.notFound")}</h1>
                    <Link to="/guest-previews" className="mt-6 inline-flex items-center gap-2 text-sm text-stone-600 hover:text-stone-950 dark:text-stone-300 dark:hover:text-stone-100">
                        <ArrowLeftOutlined /> {t("guestPreviews.backToList")}
                    </Link>
                </section>
            </main>
        );
    }

    return (
        <main className="h-full overflow-y-auto bg-background">
            <section className="mx-auto max-w-5xl px-6 py-10">
                <Link to="/guest-previews" className="inline-flex items-center gap-2 text-sm text-stone-600 hover:text-stone-950 dark:text-stone-300 dark:hover:text-stone-100">
                    <ArrowLeftOutlined /> {t("guestPreviews.backToList")}
                </Link>

                <div className="mt-6 flex flex-wrap items-center gap-2">
                    {item.tags.map((tag) => <Tag key={tag}>{tag}</Tag>)}
                </div>
                <h1 className="mt-3 text-3xl font-semibold text-stone-950 dark:text-stone-100">{item.title}</h1>
                <p className="mt-2 text-sm text-stone-500 dark:text-stone-400">{item.location}</p>
                <p className="mt-5 max-w-3xl text-base leading-7 text-stone-700 dark:text-stone-300">{item.description}</p>

                <div className="mt-8 overflow-hidden rounded-xl border border-stone-200 bg-stone-950 dark:border-stone-800">
                    {item.mediaType === "video" && item.videoUrl ? (
                        <video
                            src={item.videoUrl}
                            poster={item.coverUrl}
                            controls
                            autoPlay
                            muted
                            loop
                            playsInline
                            preload="metadata"
                            className="max-h-[70vh] w-full object-contain"
                            aria-label={item.title}
                        />
                    ) : (
                        <img src={item.coverUrl} alt={item.title} className="max-h-[70vh] w-full object-contain" />
                    )}
                </div>

                <div className="mt-10 grid gap-6 md:grid-cols-[1.6fr_1fr]">
                    <div>
                        <h2 className="text-lg font-semibold text-stone-950 dark:text-stone-100">{t("guestPreviews.highlightsTitle")}</h2>
                        <ul className="mt-3 space-y-3 text-sm leading-6 text-stone-700 dark:text-stone-300">
                            {item.highlights.map((highlight) => (
                                <li key={highlight} className="flex gap-3">
                                    <span aria-hidden className="mt-2 size-1.5 shrink-0 rounded-full bg-stone-500" />
                                    <span>{highlight}</span>
                                </li>
                            ))}
                        </ul>
                        <p className="mt-6 text-sm leading-6 text-stone-600 dark:text-stone-400">{item.summary}</p>
                    </div>
                    <Card size="small" title={t("guestPreviews.generationTitle")}>
                        <dl className="space-y-3 text-sm">
                            <div>
                                <dt className="text-xs text-stone-500 dark:text-stone-400">{t("guestPreviews.generationModel")}</dt>
                                <dd className="mt-1 font-medium text-stone-900 dark:text-stone-100">{item.generation.model}</dd>
                            </div>
                            <div>
                                <dt className="text-xs text-stone-500 dark:text-stone-400">{t("guestPreviews.generationProvider")}</dt>
                                <dd className="mt-1 text-stone-800 dark:text-stone-200">{item.generation.provider}</dd>
                            </div>
                        </dl>
                        <p className="mt-4 text-xs leading-5 text-stone-500 dark:text-stone-400">{item.generation.note}</p>
                    </Card>
                </div>

                {item.shots.length ? (
                    <div className="mt-10">
                        <h2 className="text-lg font-semibold text-stone-950 dark:text-stone-100">{t("guestPreviews.shotsTitle")}</h2>
                        <div className="mt-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                            {item.shots.map((shot) => (
                                <figure key={shot.url} className="overflow-hidden rounded-lg border border-stone-200 dark:border-stone-800">
                                    <img src={shot.url} alt={shot.caption} className="aspect-[3/2] w-full object-cover" />
                                    <figcaption className="px-3 py-2 text-xs text-stone-500 dark:text-stone-400">{shot.caption}</figcaption>
                                </figure>
                            ))}
                        </div>
                    </div>
                ) : null}

                {item.mediaType === "video" && item.videoUrl ? (
                    <p className="mt-8 inline-flex items-center gap-2 text-xs text-stone-500 dark:text-stone-400">
                        <PlayCircleOutlined /> {t("guestPreviews.videoHint")}
                    </p>
                ) : null}
            </section>
        </main>
    );
}
