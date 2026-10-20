import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";
export default defineConfig({
    base: "/",
    plugins: [react()],
    resolve: { alias: { "@": resolve(import.meta.dirname, "src") } },
    define: { __APP_VERSION__: JSON.stringify("PDF Master V0.1.7"), __APP_RELEASES__: "[]" },
    build: { outDir: "../Resources/CanvasWeb", emptyOutDir: true },
});
