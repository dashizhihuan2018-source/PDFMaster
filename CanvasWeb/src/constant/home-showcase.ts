/** Import your own, licensed media; no bundled demonstration photographs. */
export type HomeShowcaseItem = {
    id: string; title: string; prompt: string; coverUrl: string; tags: string[];
};
export const HOME_SHOWCASE: HomeShowcaseItem[] = [];
