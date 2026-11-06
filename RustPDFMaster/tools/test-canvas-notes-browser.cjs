// Compiled Rust/WASM UI with strict synthetic native replies; not native-runtime acceptance.
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const assert = require('node:assert/strict');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

const root = path.resolve(__dirname, '../dist');
const image =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U0AAAAASUVORK5CYII=';
const alternateImage =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';

const server = http.createServer((request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const file = path.resolve(root, `.${pathname}`);
  if (file !== root && !file.startsWith(`${root}${path.sep}`)) {
    response.writeHead(403);
    response.end();
    return;
  }
  const target = file === root ? path.join(root, 'index.html') : file;
  try {
    response.setHeader(
      'Content-Type',
      target.endsWith('.wasm')
        ? 'application/wasm'
        : target.endsWith('.js')
          ? 'text/javascript'
          : target.endsWith('.css')
            ? 'text/css'
            : 'text/html',
    );
    response.end(fs.readFileSync(target));
  } catch {
    response.writeHead(404);
    response.end();
  }
});

function documentView(noteEditor = null, pageCount = 2) {
  return {
    id: 7,
    name: 'synthetic.pdf',
    tabs: [{ id: 7, name: 'synthetic.pdf', dirty: false }],
    restored: false,
    info: {
      page_count: pageCount,
      pages: Array.from({ length: pageCount }, () => ({ width: 595, height: 842, rotation: 0 })),
      dirty: false,
      can_undo: false,
      can_redo: false,
    },
    ...(noteEditor ? { note_editor: noteEditor } : {}),
  };
}

function existingEditor(id, request, page = 0) {
  return {
    id,
    page,
    request,
    target: {
      kind: 'existing',
      original: {
        annotation_index: 3,
        bounds: { x: 10, y: 500, width: 24, height: 24 },
        contents: '旧便笺 🦀',
        color: { r: 1, g: 0.85, b: 0.2 },
      },
    },
  };
}

function newEditor(id, request, page = 0, x = 300, y = 400) {
  return {
    id,
    page,
    request,
    target: {
      kind: 'new',
      bounds: { x, y, width: 24, height: 24 },
      color: { r: 1, g: 0.85, b: 0.2 },
    },
  };
}

async function requestCount(page, command) {
  return page.evaluate(
    (name) => window.requests.filter((request) => request.command === name).length,
    command,
  );
}

async function requests(page, command) {
  return page.evaluate(
    (name) => window.requests.filter((request) => request.command === name),
    command,
  );
}

async function waitForRequestCount(page, command, count) {
  try {
    await page.waitForFunction(
      ({ name, expected }) =>
        window.requests.filter((request) => request.command === name).length >= expected,
      { name: command, expected: count },
    );
  } catch (error) {
    const state = await page.evaluate(() => ({
      requests: window.requests,
      status: [...document.querySelectorAll('[role=status]')].map((node) => node.textContent),
      active: [...document.querySelectorAll('[aria-pressed=true]')].map((node) =>
        node.getAttribute('aria-label'),
      ),
    }));
    throw new Error(
      `${command} did not reach ${count}: ${JSON.stringify(state)}\n${error.message}`,
    );
  }
}

async function pendingValue(page, bucket, match) {
  return page.evaluate(
    ({ bucket: name, match: expected }) => {
      const item = window.fixture[name].find(
        (candidate) =>
          !candidate.settled &&
          Object.entries(expected).every(([key, value]) => candidate.args[key] === value),
      );
      if (!item) throw new Error(`missing pending ${name}: ${JSON.stringify(expected)}`);
      return item.value;
    },
    { bucket, match },
  );
}

async function releasePending(page, bucket, match, reply) {
  await page.evaluate(
    ({ bucket: name, match: expected, reply: result }) => {
      const item = window.fixture[name].find(
        (candidate) =>
          !candidate.settled &&
          Object.entries(expected).every(([key, value]) => candidate.args[key] === value),
      );
      if (!item) throw new Error(`missing pending ${name}: ${JSON.stringify(expected)}`);
      item.settled = true;
      if (result.reject) {
        item.reject(result.reject);
        return;
      }
      if (name === 'commits') {
        const key = String(item.args.id);
        window.fixture.successfulCommits[key] =
          (window.fixture.successfulCommits[key] || 0) + 1;
      }
      if ((name === 'commits' || name === 'cancels') && result.value) {
        window.fixture.owner = result.value.note_editor || null;
      }
      item.resolve(result.value);
    },
    { bucket, match, reply },
  );
}

async function imageBox(page) {
  const value = await page.locator('#pdf-page-image').boundingBox();
  assert.ok(value && value.width > 0 && value.height > 0);
  return value;
}

async function clientPoint(page, x, y) {
  const box = await imageBox(page);
  return {
    x: box.x + (x / 595) * box.width,
    y: box.y + (1 - y / 842) * box.height,
  };
}

async function clickPoint(page, point) {
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.up();
}

async function installFixture(page, options = {}) {
  await page.addInitScript(
    ({ imageSources, restored, pageCount }) => {
      window.requests = [];
      window.fixture = {
        begins: [],
        commits: [],
        cancels: [],
        next: 10,
        notesByPage: { 0: [] },
        owner: restored,
        pageCount,
        renderSerial: 0,
        successfulCommits: {},
      };

      const view = (owner = window.fixture.owner) => ({
        id: 7,
        name: 'synthetic.pdf',
        tabs: [{ id: 7, name: 'synthetic.pdf', dirty: false }],
        restored: false,
        info: {
          page_count: window.fixture.pageCount,
          pages: Array.from({ length: window.fixture.pageCount }, () => ({
            width: 595,
            height: 842,
            rotation: 0,
          })),
          dirty: false,
          can_undo: false,
          can_redo: false,
        },
        ...(owner ? { note_editor: owner } : {}),
      });

      const pending = (bucket, command, args, value) =>
        new Promise((resolve, reject) => {
          window.fixture[bucket].push({ command, args, value, resolve, reject, settled: false });
        });

      window.__TAURI__ = {
        core: {
          invoke: async (command, rawArgs) => {
            let args = rawArgs instanceof Map ? Object.fromEntries(rawArgs) : rawArgs;
            args =
              args === undefined
                ? null
                : JSON.parse(
                    JSON.stringify(args, (_key, value) =>
                      value instanceof Map ? Object.fromEntries(value) : value,
                    ),
                  );
            window.requests.push({ command, args });

            if (command === 'document_info') return view();
            if (command === 'render_page' || command === 'render_overview_page') {
              window.fixture.renderSerial += 1;
              return imageSources[window.fixture.renderSerial % imageSources.length];
            }
            if (command === 'notes_document') {
              return {
                notes: window.fixture.notesByPage[String(args.page)] || [],
                unsupported_count: 0,
              };
            }
            if (command === 'shapes_document') return { shapes: [], unsupported_count: 0 };
            if (command === 'annotations_document' || command === 'outline_document') return [];
            if (command === 'register_file_drop_channel') return 1;
            if (command === 'prepare_page_overview') return 1;
            if (command === 'begin_canvas_note_editor') {
              const id = ++window.fixture.next;
              let target;
              if (args.target.kind === 'existing') {
                const original = (window.fixture.notesByPage[String(args.page)] || []).find(
                  (note) => note.annotation_index === args.target.annotation_index,
                );
                if (!original || original.contents !== args.target.expected_contents) {
                  throw new Error('strict fixture could not bind existing note');
                }
                target = { kind: 'existing', original };
              } else {
                target = {
                  kind: 'new',
                  bounds: {
                    x: Math.max(0, Math.min(args.target.page_width - 24, args.target.x - 12)),
                    y: Math.max(0, Math.min(args.target.page_height - 24, args.target.y - 12)),
                    width: 24,
                    height: 24,
                  },
                  color: { r: 1, g: 0.85, b: 0.2 },
                };
              }
              const editor = { id, page: args.page, request: args.request, target };
              return pending('begins', command, args, editor);
            }
            if (command === 'commit_note_editor') {
              return pending('commits', command, args, null);
            }
            if (command === 'cancel_canvas_note_request' || command === 'cancel_note_editor') {
              return pending('cancels', command, args, null);
            }
            throw new Error(`unexpected fixture command ${command}`);
          },
        },
      };
    },
    {
      imageSources: [image, alternateImage],
      restored: options.restored || null,
      pageCount: options.pageCount || 2,
    },
  );
}

async function openFixture(browser, width, options = {}) {
  const context = await browser.newContext({ viewport: { width, height: 900 } });
  const page = await context.newPage();
  page.setDefaultTimeout(5000);
  const errors = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  await installFixture(page, options);
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.getByRole('button', { name: '文档标签 synthetic.pdf', exact: true }).waitFor();
  await page.locator('#pdf-page-image').waitFor();
  return { context, page, errors };
}

async function activateToolbar(page) {
  await page.getByRole('button', { name: '批注', exact: true }).click();
  await page.getByRole('button', { name: '工具 · 便笺', exact: true }).click();
  assert.equal(await page.locator('.inspector:visible').count(), 0);
  assert.equal(await page.locator('#tool-notes').count(), 0);
  assert.equal(await page.getByRole('button', { name: '添加便笺到左上方', exact: true }).count(), 0);
}

async function assertCancelledPointerPaths(page) {
  const base = await requestCount(page, 'begin_canvas_note_editor');
  const box = await imageBox(page);
  const point = { x: box.x + box.width * 0.55, y: box.y + box.height * 0.45 };

  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.move(point.x + 8, point.y);
  await page.mouse.move(point.x, point.y);
  await page.mouse.up();
  assert.equal(await requestCount(page, 'begin_canvas_note_editor'), base);

  for (const eventName of ['pointercancel', 'lostpointercapture']) {
    await page.mouse.move(point.x, point.y);
    await page.mouse.down();
    await page.locator('.drawing-surface').dispatchEvent(eventName, {
      pointerId: 1,
      isPrimary: true,
      button: 0,
      clientX: point.x,
      clientY: point.y,
    });
    await page.mouse.up();
    assert.equal(await requestCount(page, 'begin_canvas_note_editor'), base, eventName);
  }

  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.locator('#pdf-page-image').evaluate((node) => {
    node.style.transform = 'scale(.98)';
  });
  await page.mouse.up();
  await page.locator('#pdf-page-image').evaluate((node) => {
    node.style.transform = '';
  });
  assert.equal(await requestCount(page, 'begin_canvas_note_editor'), base);

  let renders = await requestCount(page, 'render_page');
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.getByRole('combobox', { name: '缩放', exact: true }).selectOption('1.25');
  await waitForRequestCount(page, 'render_page', renders + 1);
  await page.locator('#pdf-page-image').waitFor();
  await page.mouse.up();
  assert.equal(await requestCount(page, 'begin_canvas_note_editor'), base);

  renders = await requestCount(page, 'render_page');
  const current = await imageBox(page);
  await page.mouse.move(current.x + current.width * 0.5, current.y + current.height * 0.5);
  await page.mouse.down();
  await page.getByRole('button', { name: '下一页', exact: true }).evaluate((button) => button.click());
  await waitForRequestCount(page, 'render_page', renders + 1);
  await page.mouse.up();
  assert.equal(await requestCount(page, 'begin_canvas_note_editor'), base);
  const noteReads = await requestCount(page, 'notes_document');
  await page.getByRole('button', { name: '上一页', exact: true }).click();
  await waitForRequestCount(page, 'notes_document', noteReads + 1);
  await page.waitForFunction(() => {
    const image = document.querySelector('#pdf-page-image');
    return image?.complete && image.naturalWidth > 0;
  });
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );

  const positiveBase = await requestCount(page, 'begin_canvas_note_editor');
  const positiveBox = await imageBox(page);
  const positivePoint = {
    x: positiveBox.x + positiveBox.width * 0.4,
    y: positiveBox.y + positiveBox.height * 0.4,
  };
  await page.evaluate(() => {
    window.pointerLog = [];
    const surface = document.querySelector('.drawing-surface');
    for (const name of ['pointerdown', 'pointerup', 'pointercancel', 'lostpointercapture']) {
      surface.addEventListener(
        name,
        (event) => window.pointerLog.push({ name, pointerId: event.pointerId }),
        { once: true, capture: true },
      );
    }
  });
  await clickPoint(page, positivePoint);
  try {
    await waitForRequestCount(page, 'begin_canvas_note_editor', positiveBase + 1);
  } catch (error) {
    const state = await page.evaluate(({ x, y }) => {
      const image = document.querySelector('#pdf-page-image');
      const hit = document.elementFromPoint(x, y);
      return {
        pointerLog: window.pointerLog,
        hit: {
          tag: hit?.tagName || null,
          id: hit?.id || null,
          className: hit?.getAttribute('class') || null,
        },
        image: image
          ? {
              bounds: image.getBoundingClientRect().toJSON(),
              src: image.getAttribute('src'),
              currentSrc: image.currentSrc,
              owner: image.getAttribute('data-annotation-owner'),
              complete: image.complete,
              naturalWidth: image.naturalWidth,
            }
          : null,
      };
    }, positivePoint);
    throw new Error(`valid click after invalidations failed: ${JSON.stringify(state)}\n${error.message}`);
  }
  const positiveBegin = (await requests(page, 'begin_canvas_note_editor')).at(-1);
  const positiveEditor = await pendingValue(page, 'begins', {
    request: positiveBegin.args.request,
  });
  const positiveBody = await acknowledgeBegin(page, positiveBegin, positiveEditor);
  const positiveCancels = await requestCount(page, 'cancel_note_editor');
  await positiveBody.press('Escape');
  await waitForRequestCount(page, 'cancel_note_editor', positiveCancels + 1);
  await releasePending(page, 'cancels', { id: positiveEditor.id }, { value: documentView() });
  await positiveBody.waitFor({ state: 'hidden' });
}

async function beginAt(page, point) {
  await page.waitForFunction(
    () => document.querySelector('#pdf-reader')?.getAttribute('aria-busy') === 'false',
  );
  await page.waitForFunction(() => {
    const image = document.querySelector('#pdf-page-image');
    return image?.complete && image.naturalWidth > 0;
  });
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
  assert.equal(
    await page.getByRole('button', { name: '工具 · 便笺', exact: true }).isDisabled(),
    false,
    'Note tool must not remain blocked before the next pointer',
  );
  const hit = await page.evaluate(
    ({ x, y }) => {
      const node = document.elementFromPoint(x, y);
      return {
        tag: node?.tagName || null,
        id: node?.id || null,
        className: node?.getAttribute('class') || null,
        inSurface: Boolean(node?.closest('.drawing-surface')),
      };
    },
    point,
  );
  assert.equal(
    hit.inSurface,
    true,
    `pointer missed drawing surface: ${JSON.stringify({ point, hit })}`,
  );
  const before = await requestCount(page, 'begin_canvas_note_editor');
  await clickPoint(page, point);
  await waitForRequestCount(page, 'begin_canvas_note_editor', before + 1);
  const begin = (await requests(page, 'begin_canvas_note_editor')).at(-1);
  const editor = await pendingValue(page, 'begins', { request: begin.args.request });
  return { begin, editor };
}

async function beginAtPagePoint(page, x, y) {
  await page.waitForFunction(
    () => document.querySelector('#pdf-reader')?.getAttribute('aria-busy') === 'false',
  );
  return beginAt(page, await clientPoint(page, x, y));
}

async function acknowledgeBegin(page, begin, editor) {
  await releasePending(page, 'begins', { request: begin.args.request }, { value: editor });
  const body = page.getByRole('textbox', { name: '便笺正文', exact: true });
  await body.waitFor();
  await page.waitForFunction(
    () => document.activeElement?.getAttribute('aria-label') === '便笺正文',
  );
  return body;
}

async function attemptNativeTextareaResize(page, body) {
  const before = await body.boundingBox();
  const viewport = page.viewportSize();
  assert.ok(before && viewport);
  const start = {
    x: before.x + before.width - 2,
    y: before.y + before.height - 2,
  };
  const target = {
    x: Math.min(viewport.width - 2, start.x + 48),
    y: Math.min(viewport.height - 2, start.y + 220),
  };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(target.x, target.y, { steps: 12 });
  await page.mouse.up();
  await page.waitForTimeout(50);
  return { before, after: await body.boundingBox(), viewport };
}

async function runWidth(browser, width) {
  const pointerFixture = await openFixture(browser, width);
  await activateToolbar(pointerFixture.page);
  await assertCancelledPointerPaths(pointerFixture.page);
  assert.equal(pointerFixture.errors.length, 0, pointerFixture.errors.join('\n'));
  await pointerFixture.context.close();

  const { context, page, errors } = await openFixture(browser, width);
  await activateToolbar(page);

  const { begin, editor } = await beginAtPagePoint(page, 357, 505.2);
  const before = await imageBox(page);
  assert.match(begin.args.request, /^[0-9a-f]{32}$/);
  assert.equal(begin.args.target.kind, 'new');
  assert.ok(Math.abs(begin.args.target.x - 357) < 5, JSON.stringify(begin.args.target));
  assert.ok(Math.abs(begin.args.target.y - 505.2) < 5, JSON.stringify(begin.args.target));
  assert.notEqual(Math.round(begin.args.target.x), 36);
  assert.equal(await page.getByRole('textbox', { name: '便笺正文', exact: true }).count(), 0);

  const body = await acknowledgeBegin(page, begin, editor);
  assert.equal(await page.getByText(/#[0-9A-Fa-f]{6}/).count(), 0);
  assert.equal(await page.getByText(/PDF 坐标/).count(), 0);
  const editorBox = await page.locator('.canvas-note-editor').boundingBox();
  assert.ok(
    editorBox &&
      editorBox.x >= 0 &&
      editorBox.y >= 0 &&
      editorBox.x + editorBox.width <= width + 0.5 &&
      editorBox.y + editorBox.height <= 900.5,
  );
  assert.equal(
    await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth),
    true,
  );
  assert.deepEqual(await imageBox(page), before);

  await body.fill('Unicode 正文 🦀');
  await body.evaluate((element) => {
    for (const event of [
      new KeyboardEvent('keydown', { key: 'Escape', isComposing: true, bubbles: true }),
      new KeyboardEvent('keydown', {
        key: 'Enter',
        ctrlKey: true,
        isComposing: true,
        bubbles: true,
      }),
    ]) {
      element.dispatchEvent(event);
    }
  });
  assert.equal(await body.count(), 1);
  assert.equal(await requestCount(page, 'commit_note_editor'), 0);

  await body.press('Control+Enter');
  await waitForRequestCount(page, 'commit_note_editor', 1);
  assert.deepEqual((await requests(page, 'commit_note_editor')).at(-1).args, {
    id: editor.id,
    contents: 'Unicode 正文 🦀',
  });
  await releasePending(page, 'commits', { id: editor.id }, { reject: 'synthetic finish failure' });
  await page.waitForFunction(() => document.body.textContent.includes('正文与原生ID已保留'));
  assert.equal(await body.inputValue(), 'Unicode 正文 🦀');
  await body.fill('失败后可见文本发生变化');
  await page.getByRole('button', { name: '重试完成便笺', exact: true }).click();
  await waitForRequestCount(page, 'commit_note_editor', 2);
  assert.deepEqual(
    (await requests(page, 'commit_note_editor')).at(-1).args,
    { id: editor.id, contents: 'Unicode 正文 🦀' },
    '失败普通提交必须精确重放原 payload',
  );
  await releasePending(page, 'commits', { id: editor.id }, { value: documentView() });
  await body.waitFor({ state: 'hidden' });

  const emptyFlow = await beginAtPagePoint(page, 200, 300);
  await acknowledgeBegin(page, emptyFlow.begin, emptyFlow.editor);
  await page.getByRole('button', { name: '完成', exact: true }).click();
  await waitForRequestCount(page, 'commit_note_editor', 3);
  assert.deepEqual((await requests(page, 'commit_note_editor')).at(-1).args, {
    id: emptyFlow.editor.id,
    contents: '',
  });
  await releasePending(page, 'commits', { id: emptyFlow.editor.id }, { value: documentView() });
  await body.waitFor({ state: 'hidden' });

  const metaFlow = await beginAtPagePoint(page, 250, 350);
  const metaBody = await acknowledgeBegin(page, metaFlow.begin, metaFlow.editor);
  await metaBody.fill('Meta 完成');
  const beforeMeta = await requestCount(page, 'commit_note_editor');
  await metaBody.press('Meta+Enter');
  await waitForRequestCount(page, 'commit_note_editor', beforeMeta + 1);
  assert.deepEqual((await requests(page, 'commit_note_editor')).at(-1).args, {
    id: metaFlow.editor.id,
    contents: 'Meta 完成',
  });
  await releasePending(page, 'commits', { id: metaFlow.editor.id }, { value: documentView() });
  await metaBody.waitFor({ state: 'hidden' });

  const cancelFlow = await beginAtPagePoint(page, 300, 420);
  const cancelBody = await acknowledgeBegin(page, cancelFlow.begin, cancelFlow.editor);
  await cancelBody.fill('取消失败仍保留');
  let cancelCount = await requestCount(page, 'cancel_note_editor');
  await page.getByRole('button', { name: '取消', exact: true }).click();
  await waitForRequestCount(page, 'cancel_note_editor', cancelCount + 1);
  await releasePending(
    page,
    'cancels',
    { id: cancelFlow.editor.id },
    { reject: 'synthetic cancel failure' },
  );
  await page.waitForFunction(() => document.body.textContent.includes('便笺恢复入口仍保留'));
  assert.equal(await cancelBody.inputValue(), '取消失败仍保留');
  const retryBefore = await requestCount(page, 'commit_note_editor');
  await page.getByRole('button', { name: '重试完成便笺', exact: true }).click();
  await waitForRequestCount(page, 'commit_note_editor', retryBefore + 1);
  assert.deepEqual((await requests(page, 'commit_note_editor')).at(-1).args, {
    id: cancelFlow.editor.id,
    contents: '取消失败仍保留',
  });
  await releasePending(page, 'commits', { id: cancelFlow.editor.id }, { value: documentView() });
  await cancelBody.waitFor({ state: 'hidden' });

  const lateFlow = await beginAtPagePoint(page, 400, 400);
  const pendingCancels = await requestCount(page, 'cancel_canvas_note_request');
  await page.getByRole('button', { name: '取消或同步便笺', exact: true }).click();
  await waitForRequestCount(page, 'cancel_canvas_note_request', pendingCancels + 1);
  await releasePending(
    page,
    'cancels',
    { request: lateFlow.begin.args.request },
    { value: documentView() },
  );
  await releasePending(
    page,
    'begins',
    { request: lateFlow.begin.args.request },
    { value: lateFlow.editor },
  );
  await waitForRequestCount(page, 'cancel_canvas_note_request', pendingCancels + 2);
  await releasePending(
    page,
    'cancels',
    { request: lateFlow.begin.args.request },
    { value: documentView() },
  );
  assert.equal(await body.count(), 0);

  // A bounded bridge timeout abandons observation of the losing invoke. Reconcile the
  // finished old native id, establish a newer owner through the real pointer/begin path,
  // and start its terminal attempt before delivering the abandoned old transport reply.
  const oldFlow = await beginAtPagePoint(page, 450, 450);
  const oldBody = await acknowledgeBegin(page, oldFlow.begin, oldFlow.editor);
  await oldBody.fill('旧 owner 终态');
  const ownerCommits = await requestCount(page, 'commit_note_editor');
  await oldBody.press('Control+Enter');
  await waitForRequestCount(page, 'commit_note_editor', ownerCommits + 1);
  await page.waitForFunction(
    () =>
      document.body.textContent.includes('原生响应超时') &&
      document.body.textContent.includes('正文与原生ID已保留'),
    null,
    { timeout: 18_000 },
  );
  assert.equal(await oldBody.inputValue(), '旧 owner 终态');

  cancelCount = await requestCount(page, 'cancel_note_editor');
  await page.getByRole('button', { name: '取消或同步便笺', exact: true }).click();
  await waitForRequestCount(page, 'cancel_note_editor', cancelCount + 1);
  await releasePending(page, 'cancels', { id: oldFlow.editor.id }, { value: documentView() });
  await oldBody.waitFor({ state: 'hidden' });

  await page.getByRole('button', { name: '下一页', exact: true }).click();
  const newerFlow = await beginAtPagePoint(page, 420, 300);
  const newerBody = await acknowledgeBegin(page, newerFlow.begin, newerFlow.editor);
  assert.equal(newerFlow.editor.page, 1);
  assert.equal(newerFlow.editor.request, newerFlow.begin.args.request);
  await newerBody.fill('新 owner 精确终态 🦀');
  const newerStyle = await page.locator('.canvas-note-editor').getAttribute('style');
  assert.ok(newerStyle && !newerStyle.includes('--note-x:24px;--note-y:24px'), newerStyle);
  const newerCommits = await requestCount(page, 'commit_note_editor');
  await newerBody.press('Control+Enter');
  await waitForRequestCount(page, 'commit_note_editor', newerCommits + 1);
  assert.deepEqual((await requests(page, 'commit_note_editor')).at(-1).args, {
    id: newerFlow.editor.id,
    contents: '新 owner 精确终态 🦀',
  });

  // This resolution is the losing invoke after Promise.race already settled. It must
  // not execute another WASM completion branch or disturb the newer local owner.
  await releasePending(
    page,
    'commits',
    { id: oldFlow.editor.id },
    { value: documentView() },
  );
  await page.evaluate(() => new Promise((resolve) => setTimeout(resolve, 0)));
  assert.equal(await newerBody.inputValue(), '新 owner 精确终态 🦀');
  assert.equal(await page.getByLabel('跳转页码').inputValue(), '2');
  assert.equal(await page.locator('.canvas-note-editor').getAttribute('style'), newerStyle);
  assert.equal(await newerBody.isDisabled(), true, '新 owner 终态仍在等待原生回复');
  assert.equal(
    await page.evaluate(
      ({ id, contents }) =>
        window.fixture.commits.some(
          (item) => !item.settled && item.args.id === id && item.args.contents === contents,
        ),
      { id: newerFlow.editor.id, contents: '新 owner 精确终态 🦀' },
    ),
    true,
    '新 owner 的原生 id 与精确终态必须保持 pending',
  );

  await releasePending(
    page,
    'commits',
    { id: newerFlow.editor.id },
    { reject: 'synthetic newer finish failure' },
  );
  await page.waitForFunction(() => document.body.textContent.includes('正文与原生ID已保留'));
  cancelCount = await requestCount(page, 'cancel_note_editor');
  await page.getByRole('button', { name: '取消或同步便笺', exact: true }).click();
  await waitForRequestCount(page, 'cancel_note_editor', cancelCount + 1);
  await releasePending(
    page,
    'cancels',
    { id: newerFlow.editor.id },
    { value: documentView(newerFlow.editor) },
  );
  await page.waitForFunction(() => document.body.textContent.includes('原生仍有便笺owner'));
  const retryNewer = await requestCount(page, 'commit_note_editor');
  await page.getByRole('button', { name: '重试完成便笺', exact: true }).click();
  await waitForRequestCount(page, 'commit_note_editor', retryNewer + 1);
  assert.deepEqual((await requests(page, 'commit_note_editor')).at(-1).args, {
    id: newerFlow.editor.id,
    contents: '新 owner 精确终态 🦀',
  });
  await releasePending(
    page,
    'commits',
    { id: newerFlow.editor.id },
    { value: documentView() },
  );
  await newerBody.waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: '上一页', exact: true }).click();

  // Retain the separate authoritative-other-owner reconciliation coverage. This
  // is not the stale Promise.race proof above: this reply itself carries the owner.
  const ownerFlow = await beginAtPagePoint(page, 450, 450);
  const ownerBody = await acknowledgeBegin(page, ownerFlow.begin, ownerFlow.editor);
  await ownerBody.fill('旧 owner 提交');
  const authoritativeCommits = await requestCount(page, 'commit_note_editor');
  await ownerBody.press('Control+Enter');
  await waitForRequestCount(page, 'commit_note_editor', authoritativeCommits + 1);
  const authoritativeNewer = newEditor(900, 'restored-newer-owner', 1, 420, 300);
  await releasePending(
    page,
    'commits',
    { id: ownerFlow.editor.id },
    { value: documentView(authoritativeNewer) },
  );
  await page.waitForFunction(() => document.body.textContent.includes('原生返回了另一个便笺owner'));
  assert.equal(await ownerBody.inputValue(), '');
  assert.equal(await page.getByLabel('跳转页码').inputValue(), '2');
  const ownerStyle = await page.locator('.canvas-note-editor').getAttribute('style');
  assert.ok(ownerStyle && !ownerStyle.includes('--note-x:24px;--note-y:24px'), ownerStyle);
  cancelCount = await requestCount(page, 'cancel_note_editor');
  await ownerBody.press('Escape');
  await waitForRequestCount(page, 'cancel_note_editor', cancelCount + 1);
  await releasePending(
    page,
    'cancels',
    { id: authoritativeNewer.id },
    { value: documentView() },
  );
  await ownerBody.waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: '上一页', exact: true }).click();

  await page.evaluate(() => {
    window.fixture.notesByPage['0'] = [
      {
        annotation_index: 3,
        bounds: { x: 10, y: 500, width: 24, height: 24 },
        contents: '旧便笺 🦀',
        color: { r: 1, g: 0.85, b: 0.2 },
      },
    ];
  });
  const noteReads = await requestCount(page, 'notes_document');
  await page.getByRole('combobox', { name: '缩放', exact: true }).selectOption('1.5');
  await waitForRequestCount(page, 'notes_document', noteReads + 1);
  const existingFlow = await beginAtPagePoint(page, 22, 512);
  assert.deepEqual(existingFlow.begin.args.target, {
    kind: 'existing',
    annotation_index: 3,
    expected_contents: '旧便笺 🦀',
  });
  const existingBody = await acknowledgeBegin(page, existingFlow.begin, existingFlow.editor);
  assert.equal(await existingBody.inputValue(), '旧便笺 🦀');
  const deleteCommits = await requestCount(page, 'commit_note_editor');
  await page.getByRole('button', { name: '删除便笺', exact: true }).click();
  await waitForRequestCount(page, 'commit_note_editor', deleteCommits + 1);
  assert.deepEqual((await requests(page, 'commit_note_editor')).at(-1).args, {
    id: existingFlow.editor.id,
    contents: '',
  });
  await releasePending(
    page,
    'commits',
    { id: existingFlow.editor.id },
    { reject: 'synthetic delete failure' },
  );
  await page.waitForFunction(() => document.body.textContent.includes('正文与原生ID已保留'));
  assert.equal(await existingBody.inputValue(), '旧便笺 🦀');
  await page.getByRole('button', { name: '重试完成便笺', exact: true }).click();
  await waitForRequestCount(page, 'commit_note_editor', deleteCommits + 2);
  assert.deepEqual(
    (await requests(page, 'commit_note_editor')).at(-1).args,
    { id: existingFlow.editor.id, contents: '' },
    '删除失败后必须使用同 ID 重放精确空 payload',
  );
  await releasePending(page, 'commits', { id: existingFlow.editor.id }, { value: documentView() });
  await existingBody.waitFor({ state: 'hidden' });
  assert.equal(
    await page.evaluate(
      (id) => window.fixture.successfulCommits[String(id)] || 0,
      existingFlow.editor.id,
    ),
    1,
    'synthetic fixture must acknowledge one terminal mutation only',
  );

  await page.locator('#pdf-page-image').evaluate((image) => {
    image.scrollIntoView({ block: 'end', inline: 'nearest' });
  });
  const edgeFrame = await imageBox(page);
  const readerFrame = await page.locator('#pdf-reader').boundingBox();
  assert.ok(readerFrame);
  const viewport = page.viewportSize();
  assert.ok(viewport);
  const visibleTop = Math.max(edgeFrame.y, readerFrame.y, 0);
  const visibleBottom = Math.min(
    edgeFrame.y + edgeFrame.height,
    readerFrame.y + readerFrame.height,
    viewport.height,
  );
  const visibleLeft = Math.max(edgeFrame.x, readerFrame.x, 0);
  const visibleRight = Math.min(
    edgeFrame.x + edgeFrame.width,
    readerFrame.x + readerFrame.width,
    viewport.width,
  );
  assert.ok(
    visibleBottom > visibleTop && visibleRight > visibleLeft,
    `page bottom has no visible reader intersection: ${JSON.stringify({ edgeFrame, readerFrame, viewport })}`,
  );
  const edgePoint = {
    x: (visibleLeft + visibleRight) / 2,
    y: Math.max(visibleTop + 1, visibleBottom - 2),
  };
  const edgeFlow = await beginAt(page, edgePoint);
  const edgeBody = await acknowledgeBegin(page, edgeFlow.begin, edgeFlow.editor);
  await edgeBody.fill(
    Array.from({ length: 80 }, (_, index) => `scrollable note line ${index + 1}`).join('\n'),
  );
  const resizedBody = await attemptNativeTextareaResize(page, edgeBody);
  const edgeBox = await page.locator('.canvas-note-editor').boundingBox();
  const completeBox = await page
    .getByRole('button', { name: '完成便笺', exact: true })
    .boundingBox();
  const actionBox = await page.locator('.canvas-note-actions').boundingBox();
  const resizeBehavior = await edgeBody.evaluate((element) => ({
    resize: getComputedStyle(element).resize,
    scrollable: element.scrollHeight > element.clientHeight,
  }));
  assert.ok(
    edgeBox &&
      edgeBox.x >= 0 &&
      edgeBox.y >= 0 &&
      edgeBox.x + edgeBox.width <= width + 0.5 &&
      edgeBox.y + edgeBox.height <= 900.5,
    `native textarea resize escaped viewport: ${JSON.stringify({ edgeBox, resizedBody, resizeBehavior })}`,
  );
  for (const [name, box] of [
    ['completion', completeBox],
    ['actions', actionBox],
  ]) {
    assert.ok(
      box && box.y >= 0 && box.y + box.height <= resizedBody.viewport.height + 0.5,
      `${name} control escaped after native resize: ${JSON.stringify({ box, resizedBody })}`,
    );
  }
  assert.equal(resizeBehavior.scrollable, true, 'long note body must retain textarea scrolling');
  assert.ok(
    resizedBody.before &&
      resizedBody.after &&
      Math.abs(resizedBody.after.height - resizedBody.before.height) < 0.5,
    `native resize attempt changed bounded textarea height: ${JSON.stringify({ resizedBody, resizeBehavior })}`,
  );
  assert.deepEqual(await imageBox(page), edgeFrame, 'bottom editor must not shift the PDF frame');
  cancelCount = await requestCount(page, 'cancel_note_editor');
  await edgeBody.press('Escape');
  await waitForRequestCount(page, 'cancel_note_editor', cancelCount + 1);
  await releasePending(page, 'cancels', { id: edgeFlow.editor.id }, { value: documentView() });
  await edgeBody.waitFor({ state: 'hidden' });

  assert.equal(errors.length, 0, errors.join('\n'));
  await context.close();
}

async function restoration(browser, editor, expectedText) {
  const { context, page, errors } = await openFixture(browser, 390, {
    restored: editor,
    pageCount: 2,
  });
  const body = page.getByRole('textbox', { name: '便笺正文', exact: true });
  await body.waitFor();
  assert.equal(await body.inputValue(), expectedText);
  assert.equal(await page.getByLabel('跳转页码').inputValue(), String(editor.page + 1));
  const style = await page.locator('.canvas-note-editor').getAttribute('style');
  assert.ok(style && !style.includes('--note-x:24px;--note-y:24px'), style);
  const cancelCount = await requestCount(page, 'cancel_note_editor');
  await body.press('Escape');
  await waitForRequestCount(page, 'cancel_note_editor', cancelCount + 1);
  await releasePending(page, 'cancels', { id: editor.id }, { value: documentView() });
  await body.waitFor({ state: 'hidden' });
  assert.equal(errors.length, 0, errors.join('\n'));
  await context.close();
}

async function annotationEmptyEntry(browser) {
  const { context, page, errors } = await openFixture(browser, 1280);
  await page.getByRole('button', { name: '侧栏批注', exact: true }).click();
  const emptyEntry = page.getByRole('button', { name: '批注工具 · 便笺', exact: true });
  await emptyEntry.waitFor();
  await emptyEntry.click();
  assert.equal(
    await page.getByRole('button', { name: '工具 · 便笺', exact: true }).getAttribute('aria-pressed'),
    'true',
  );
  assert.equal(await page.locator('.inspector:visible').count(), 0);
  assert.equal(await page.locator('#tool-notes').count(), 0);
  assert.equal(errors.length, 0, errors.join('\n'));
  await context.close();
}

(async () => {
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch({
      headless: true,
      ...(process.env.PLAYWRIGHT_EXECUTABLE_PATH
        ? { executablePath: process.env.PLAYWRIGHT_EXECUTABLE_PATH }
        : {}),
    });
    for (const width of [1280, 390]) await runWidth(browser, width);
    await restoration(browser, existingEditor(77, 'restored-existing', 1), '旧便笺 🦀');
    await restoration(browser, newEditor(78, 'restored-new', 1, 420, 300), '');
    await annotationEmptyEntry(browser);
    console.log(
      'canvas notes compiled-WASM synthetic assertions passed (1280, 390, Existing/New restoration, annotation entry)',
    );
  } finally {
    if (browser) await browser.close();
    await new Promise((resolve) => server.close(resolve));
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
