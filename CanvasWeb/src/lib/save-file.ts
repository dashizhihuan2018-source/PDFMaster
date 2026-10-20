import { saveAs as browserSaveAs } from "file-saver";
import { nativeCanvas, nativeCall } from "./pdfmaster-native";
import { message } from "antd";

/** WebKit's FileSaver fallback loses filenames; use the native save panel instead. */
export function saveAs(source: Blob | string, name = "画布导出") {
    if (!nativeCanvas()) return browserSaveAs(source, name);
    void (async () => {
        const blob = source instanceof Blob ? source : await (await fetch(source)).blob();
        if (blob.size > 200_000_000) throw new Error("单次导出超过 200 MB，请分批导出节点或资产。");
        const bytes = new Uint8Array(await blob.arrayBuffer());
        let binary = "";
        for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
        const result = await nativeCall({ operation: "export", name, body: btoa(binary) });
        if (result.error || result.status >= 400) throw new Error("导出失败，请检查文件名与磁盘空间。");
        if (result.saved) message.success("已保存到所选位置");
    })().catch(error => message.error(error instanceof Error ? error.message : "导出失败"));
}
