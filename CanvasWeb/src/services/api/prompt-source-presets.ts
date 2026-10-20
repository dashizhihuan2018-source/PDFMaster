import { nanoid } from "nanoid";

import { publicUrl } from "@/lib/public-url";

export type PromptSource = {
    id: string;
    name: string;
    url: string;
    homepage: string;
    enabled: boolean;
    builtIn: boolean;
};

export const PROMPT_REGISTRY_HOMEPAGE = "https://github.com/yukkcat/image-prompts";

export function createPromptSource(source?: Partial<PromptSource>): PromptSource {
    return {
        id: source?.id?.trim() || nanoid(),
        name: source?.name?.trim() || "",
        url: source?.url?.trim() || "",
        homepage: source?.homepage?.trim() || "",
        enabled: source?.enabled ?? true,
        builtIn: source?.builtIn ?? false,
    };
}

// Third-party prompt snapshots are not redistributed in the public source.
export const DEFAULT_PROMPT_SOURCES: PromptSource[] = [];

function registrySource(id: string, name: string, homepage: string): PromptSource {
    return { id, name, url: publicUrl(`prompt-sources/${id}.json`), homepage, enabled: true, builtIn: true };
}
