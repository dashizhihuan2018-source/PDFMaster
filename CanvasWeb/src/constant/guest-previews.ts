/** Public desktop source does not redistribute project demonstration media. */
export type GuestPreviewShot = { url: string; caption: string };
export type GuestPreview = {
    id: string; title: string; location: string; description: string; summary: string;
    coverUrl: string; videoUrl?: string; tags: string[]; mediaType: "image" | "video";
    highlights: string[]; shots: GuestPreviewShot[];
    generation: { model: string; provider: string; note: string };
};
export const GUEST_PREVIEWS: GuestPreview[] = [];
