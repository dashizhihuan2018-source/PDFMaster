"use strict";
const {
  PDFDocument,
  PDFName,
  PDFArray,
  PDFDict,
  PDFString,
  rgb,
} = require("pdf-lib");
const MAX = 256 * 1024 * 1024;
function number(x, lo, hi) {
  if (!Number.isFinite(x) || x < lo || x > hi) throw Error("参数超出范围");
  return x;
}
function indices(list, count, all = false) {
  if (
    !Array.isArray(list) ||
    !list.length ||
    list.some((x) => !Number.isInteger(x) || x < 0 || x >= count) ||
    new Set(list).size !== list.length ||
    (all && list.length !== count)
  )
    throw Error("页码或页序无效");
  return list;
}
async function load(bytes) {
  if (!(bytes instanceof Uint8Array) || !bytes.length || bytes.length > MAX)
    throw Error("PDF 体积必须在 256 MB 以内");
  const doc = await PDFDocument.load(bytes, { updateMetadata: false });
  if (!doc.getPageCount() || doc.getPageCount() > 10000)
    throw Error("PDF 页数超出范围");
  // Page-copy and rewrite do not preserve signature validity or active documents.
  if (
    doc.catalog.has(PDFName.of("Perms")) ||
    doc.catalog.has(PDFName.of("OpenAction")) ||
    doc.catalog.has(PDFName.of("AA"))
  )
    throw Error("签名或主动操作文档只允许只读查看，请保留原件");
  const acro = doc.catalog.lookupMaybe(PDFName.of("AcroForm"), PDFDict);
  if (acro && /\/FT\s*\/Sig\b/.test(acro.toString()))
    throw Error("数字签名文档不能重写");
  for (const [, obj] of doc.context.enumerateIndirectObjects())
    if (
      obj instanceof PDFDict &&
      (obj.get(PDFName.of("Type"))?.toString() === "/Sig" ||
        obj.get(PDFName.of("FT"))?.toString() === "/Sig")
    )
      throw Error("数字签名文档不能重写");
  return doc;
}
async function blank() {
  const doc = await PDFDocument.create();
  doc.addPage([595.28, 841.89]);
  return doc.save();
}
function pageCopySafe(doc) {
  for (const name of [
    "AcroForm",
    "Outlines",
    "Names",
    "StructTreeRoot",
    "PageLabels",
    "OCProperties",
  ])
    if (doc.catalog.has(PDFName.of(name)))
      throw Error(
        "包含表单、目录、附件或结构信息的 PDF 暂不支持页面结构重写；请保留原件",
      );
}
async function modify(bytes, op, args = {}) {
  const doc = await load(bytes),
    count = doc.getPageCount();
  if (["remove", "reorder", "extract", "duplicate"].includes(op))
    pageCopySafe(doc);
  if (op === "rotate") {
    const { degrees } = require("pdf-lib");
    indices(args.pages, count).forEach((i) =>
      doc
        .getPage(i)
        .setRotation(degrees((doc.getPage(i).getRotation().angle + 90) % 360)),
    );
  } else if (op === "remove") {
    indices(args.pages, count);
    if (args.pages.length >= count) throw Error("不能删除全部页面");
    [...args.pages].sort((a, b) => b - a).forEach((i) => doc.removePage(i));
  } else if (op === "reorder" || op === "extract") {
    indices(args.pages, count, op === "reorder");
    const out = await PDFDocument.create();
    for (const p of await out.copyPages(doc, args.pages)) out.addPage(p);
    out.setTitle(doc.getTitle() || "PDF Master");
    return out.save();
  } else if (op === "duplicate") {
    indices(args.pages, count);
    const pages = await doc.copyPages(doc, args.pages);
    const pairs = pages
      .map((page, n) => ({ page, index: args.pages[n] }))
      .sort((a, b) => b.index - a.index);
    for (const item of pairs) doc.insertPage(item.index + 1, item.page);
  } else if (op === "annotate") {
    const page = doc.getPage(indices([args.page], count)[0]);
    const [w, h] = [page.getWidth(), page.getHeight()];
    const box = args.rect;
    if (!Array.isArray(box) || box.length !== 4) throw Error("无效批注范围");
    const [x0, y0, x1, y1] = box.map((v, i) => number(v, 0, i % 2 ? h : w));
    if (x1 <= x0 || y1 <= y0) throw Error("批注范围为空");
    if (
      !["Square", "Circle", "Highlight", "Ink", "FreeText", "Text"].includes(
        args.kind,
      )
    )
      throw Error("不支持的批注类型");
    const color = args.kind === "Highlight" ? [1, 0.85, 0] : [0.15, 0.32, 0.8];
    const c = doc.context;
    let content = `${color.join(" ")} RG 2 w ${x0} ${y0} ${x1 - x0} ${y1 - y0} re S`;
    if (args.kind === "Highlight")
      content = `1 .9 .3 rg ${x0} ${y0} ${x1 - x0} ${y1 - y0} re f`;
    if (args.kind === "Circle") {
      const cx = (x0 + x1) / 2,
        cy = (y0 + y1) / 2,
        rx = (x1 - x0) / 2,
        ry = (y1 - y0) / 2,
        k = 0.55228475;
      content = `${color.join(" ")} RG 2 w ${cx + rx} ${cy} m ${cx + rx} ${cy + k * ry} ${cx + k * rx} ${cy + ry} ${cx} ${cy + ry} c ${cx - k * rx} ${cy + ry} ${cx - rx} ${cy + k * ry} ${cx - rx} ${cy} c ${cx - rx} ${cy - k * ry} ${cx - k * rx} ${cy - ry} ${cx} ${cy - ry} c ${cx + k * rx} ${cy - ry} ${cx + rx} ${cy - k * ry} ${cx + rx} ${cy} c S`;
    }
    const props = {
      Type: "Annot",
      Subtype: args.kind,
      Rect: box,
      C: color,
      F: 4,
      Border: [0, 0, 2],
      NM: PDFString.of(require("node:crypto").randomUUID()),
      T: PDFString.of("Nankong"),
      Contents: require("pdf-lib").PDFHexString.fromText(
        String(args.text || "").slice(0, 8192),
      ),
    };
    if (args.kind === "Highlight") {
      props.QuadPoints = [x0, y1, x1, y1, x0, y0, x1, y0];
      props.CA = 0.35;
    }
    if (args.kind === "Ink") {
      if (
        !Array.isArray(args.points) ||
        args.points.length < 2 ||
        args.points.length > 20000
      )
        throw Error("无效手绘点");
      const pts = args.points.map((p) => [
        number(p[0], 0, w),
        number(p[1], 0, h),
      ]);
      props.InkList = [pts.flat()];
      content =
        `${color.join(" ")} RG 2 w ${pts[0].join(" ")} m ` +
        pts
          .slice(1)
          .map((p) => p.join(" ") + " l")
          .join(" ") +
        " S";
    }
    if (args.kind === "FreeText")
      throw Error("自由文字的字体嵌入尚未通过 Windows 验收；可先使用便笺评论");
    const ap = c.register(
      c.flateStream(content, {
        Type: "XObject",
        Subtype: "Form",
        BBox: box,
        Resources: {},
      }),
    );
    props.AP = { N: ap };
    const annot = c.register(c.obj(props));
    let annots = page.node.lookupMaybe(PDFName.of("Annots"), PDFArray);
    if (!annots) {
      annots = c.obj([]);
      page.node.set(PDFName.of("Annots"), annots);
    }
    annots.push(annot);
  } else if (op === "deleteAnnotation") {
    const page = doc.getPage(indices([args.page], count)[0]);
    const annots = page.node.lookupMaybe(PDFName.of("Annots"), PDFArray);
    const match =
      typeof args.ref === "string" && args.ref.match(/^(\d+)R(\d*)$/);
    if (!annots || !match) throw Error("批注引用已失效，未删除");
    const wanted = `${Number(match[1])} ${Number(match[2] || 0)} R`;
    let index = -1;
    for (let i = 0; i < annots.size(); i++)
      if (annots.get(i).toString() === wanted) index = i;
    if (index < 0) throw Error("批注已失效，未删除");
    annots.remove(index);
  } else if (op === "pageNumbers") {
    doc
      .getPages()
      .forEach((p, i) =>
        p.drawText(String(i + 1), {
          x: p.getWidth() / 2 - 4,
          y: 18,
          size: 10,
          color: rgb(0.3, 0.3, 0.3),
        }),
      );
  } else throw Error("此 Windows 功能尚未完成适配");
  return doc.save();
}
async function merge(buffers) {
  if (!Array.isArray(buffers) || buffers.length < 2 || buffers.length > 100)
    throw Error("请选择 2–100 个 PDF");
  const out = await PDFDocument.create();
  let total = 0;
  for (const data of buffers) {
    const d = await load(data);
    pageCopySafe(d);
    total += d.getPageCount();
    if (total > 10000) throw Error("页数超限");
    for (const p of await out.copyPages(d, d.getPageIndices())) out.addPage(p);
  }
  if (out.getPageCount() > 10000) throw Error("页数超限");
  return out.save();
}
async function images(buffers) {
  const d = await PDFDocument.create();
  for (const b of buffers) {
    if (b.length > 50 * 1024 * 1024) throw Error("单张图片不得超过 50 MB");
    const img = b[0] === 137 ? await d.embedPng(b) : await d.embedJpg(b);
    if (img.width * img.height > 50000000) throw Error("图片像素超限");
    const p = d.addPage([595.28, 841.89]);
    const size = img.scale(Math.min(555 / img.width, 802 / img.height));
    p.drawImage(img, {
      x: (p.getWidth() - size.width) / 2,
      y: (p.getHeight() - size.height) / 2,
      width: size.width,
      height: size.height,
    });
  }
  return d.save();
}
module.exports = { load, blank, modify, merge, images, indices, MAX };
