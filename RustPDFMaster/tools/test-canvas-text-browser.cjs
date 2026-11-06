// Actual compiled Rust/WASM DOM with synthetic D1/D2a IPC; not native QA.
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../dist');
const evidence = fs.mkdtempSync(path.join(os.tmpdir(), 'pdfmaster-canvas-text-ui-'));
const image = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U0AAAAASUVORK5CYII=';
const server = http.createServer((request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const target = path.resolve(root, `.${pathname === '/' ? '/index.html' : pathname}`);
  if (!target.startsWith(`${root}${path.sep}`)) { response.writeHead(403); response.end(); return; }
  try {
    response.setHeader('Content-Type', target.endsWith('.wasm') ? 'application/wasm' : target.endsWith('.js') ? 'text/javascript' : target.endsWith('.css') ? 'text/css' : target.endsWith('.otf') ? 'font/otf' : 'text/html');
    response.end(fs.readFileSync(target));
  } catch { response.writeHead(404); response.end(); }
});
async function fixture(page, options = {}) {
  await page.addInitScript(({ image, options }) => {
    window.requests = [];
    window.fixture = { owner: options.owner || null, options, pending: [], next: 10, activeId: 7, committed: {}, writes: 0 };
    if(options.stageTimers){
      const schedule=window.setTimeout.bind(window),clear=window.clearTimeout.bind(window);window.fixture.timers=new Map();
      window.setTimeout=(callback,delay,...args)=>{const id=schedule(callback,delay,...args);if(delay===15000)window.fixture.timers.set(id,callback);return id;};
      window.clearTimeout=id=>{window.fixture.timers.delete(id);return clear(id);};
    }
    // Actual decoded raster aspect matches PDF/display rotation. A 1×1 PNG
    // would turn height:auto into a square and hide distorted text matrices.
    const raster = includeText => {
      const canvas = document.createElement('canvas');
      const rotation = options.rotation || 0;
      canvas.width = rotation % 180 ? 842 : 595;
      canvas.height = rotation % 180 ? 595 : 842;
      const ctx = canvas.getContext('2d');
      if (rotation === 90) { ctx.translate(842,0); ctx.rotate(Math.PI/2); }
      if (rotation === 180) { ctx.translate(595,842); ctx.rotate(Math.PI); }
      if (rotation === 270) { ctx.translate(0,595); ctx.rotate(-Math.PI/2); }
      ctx.fillStyle = '#edf4f8'; ctx.fillRect(0,0,595,842);
      ctx.fillStyle = '#c7e1ed'; ctx.fillRect(0,0,595,78);
      ctx.fillStyle = '#e7cca2'; ctx.fillRect(0,730,595,112);
      ctx.strokeStyle = '#b3cbd6'; ctx.lineWidth = 1;
      for (let x=35;x<595;x+=70) { ctx.beginPath();ctx.moveTo(x,100);ctx.lineTo(x,710);ctx.stroke(); }
      ctx.fillStyle = '#485d6c';ctx.font = '16px sans-serif';ctx.fillText('Synthetic IPC · 595 × 842 PDF pt', 25, 45);
      if (includeText && options.existing) { ctx.fillStyle='#000';ctx.font='18px sans-serif';ctx.fillText(options.existing.contents,options.existing.bounds.x,842-options.existing.bounds.y-options.existing.bounds.height+18); }
      return canvas.toDataURL('image/png');
    };
    const view = () => ({ id: window.fixture.activeId, name: 'synthetic.pdf', tabs: [{ id: 7, name: 'synthetic.pdf', dirty: false }, { id: 8, name: 'second.pdf', dirty: false }], restored: false, free_text_editor: window.fixture.owner,
      info: { page_count: 2, pages: [0,1].map(() => ({ width: 595, height: 842, rotation: options.rotation || 0 })), dirty: false, can_undo: false, can_redo: false } });
    const pending = (command, args, value) => new Promise((resolve, reject) => window.fixture.pending.push({ command, args, value, resolve, reject, settled: false }));
    const outcome = (command, args, value, mode) => {
      if (mode === 'hold') return pending(command, args, value);
      if (mode === 'reject') return Promise.reject('core refusal: synthetic');
      if (mode === 'timeout') return Promise.reject('原生响应超时；结果未知');
      if (mode === 'malformed') return { invalid: true };
      if (mode === 'invalid-image') return 'data:image/png;base64,AAAA';
      return value;
    };
    window.__TAURI__ = { core: { invoke: async (command, raw) => {
      const args = JSON.parse(JSON.stringify(raw, (_key, value) => value instanceof Map ? Object.fromEntries(value) : value) || 'null');
      window.requests.push({ command, args });
      if (command === 'document_info') {window.fixture.infoCalls=(window.fixture.infoCalls||0)+1;const result=view();if(window.fixture.infoCalls>1){const conflict=window.fixture.options.syncConflict;if(conflict==='doc')result.id=88;if(conflict==='none')result.free_text_editor=null;if(conflict==='owner')result.free_text_editor={...result.free_text_editor,id:9999};}return window.fixture.infoCalls===1?result:outcome(command,args,result,window.fixture.options.sync);}
      if (command === 'render_page' || command === 'render_overview_page') {const source=raster(true);return command==='render_page'&&window.fixture.writes>0&&args.scale>.25?outcome(command,args,source,window.fixture.options.terminalRender):source;}
      if (command === 'notes_document') return { notes: [], unsupported_count: 0 };
      if (command === 'shapes_document') return { shapes: [], unsupported_count: 0 };
      if (command === 'free_texts_document') {
        if (window.fixture.options.listError) throw 'list unavailable';
        if (args.document_id !== window.fixture.activeId || !Number.isInteger(args.page)) throw 'invalid list wire';
        const list={ annotations: options.existing ? [options.existing] : [], unsupported_count: options.unsupported ? 1 : 0 };
        return window.fixture.writes>0?outcome(command,args,list,window.fixture.options.postCommitList):list;
      }
      if (command === 'annotations_document') return options.unsupported ? [options.unsupported] : [];
      if (command === 'outline_document') return [];
      if (command === 'register_file_drop_channel' || command === 'prepare_page_overview') return 1;
      if (command === 'begin_canvas_free_text_editor') {
        assertWire(args);
        const owner = { id: ++window.fixture.next, request: args.request, page: args.page, target: args.target, appearance_style: args.target.kind === 'new' ? args.target.style : (options.unknownStyle ? null : options.appearance) };
        window.fixture.owner = owner;
        return outcome(command, args, owner, window.fixture.options.begin);
      }
      if (command === 'render_free_text_editor_background') {
        if (args.id !== window.fixture.owner?.id || args.request !== window.fixture.owner?.request || !(args.scale > 0)) throw 'invalid background owner';
        return outcome(command, args, raster(false), window.fixture.options.background);
      }
      if (command === 'commit_free_text_editor') {
        const keys = Object.keys(args).sort();
        if (JSON.stringify(keys) !== JSON.stringify(['bounds','contents','id','request','style']) || typeof args.contents !== 'string') throw 'invalid commit wire';
        const previous = window.fixture.committed[args.id];
        if (previous && JSON.stringify(previous) !== JSON.stringify(args)) throw 'retry payload mismatch';
        if (!previous) { window.fixture.committed[args.id] = args; window.fixture.writes++; }
        const mode = window.fixture.options.commit;
        if (!mode) window.fixture.owner = null;
        return outcome(command, args, { ...view(), free_text_editor: null }, mode);
      }
      if (command === 'cancel_canvas_free_text_request' || command === 'cancel_free_text_editor') {
        window.fixture.owner = null;
        return outcome(command, args, view(), window.fixture.options.cancel);
      }
      if (command === 'activate_document') { window.fixture.activeId = args.id; return view(); }
      if (command === 'export_document') return view();
      if (command === 'prompt_page_action') return {document:null,target_page:null};
      if (command === 'close_document') return { closed: true, document: view() };
      throw new Error(`unexpected fixture command ${command}`);
    } } };
    function assertWire(args) {
      if (args.document_id !== window.fixture.activeId || !Number.isInteger(args.page) || !/^[a-f0-9]{32}$/.test(args.request)) throw new Error('Invalid FreeText begin wire');
      const canonical = value => JSON.stringify(value, (_k, v) => v && !Array.isArray(v) && typeof v === 'object' ? Object.fromEntries(Object.entries(v).sort(([a],[b]) => a.localeCompare(b))) : v);
      if (args.target.kind === 'existing' && canonical(args.target.original) !== canonical(options.existing)) throw new Error('Lost annotation identity');
    }
  }, { image, options });
}
async function count(page, command) { return page.evaluate(c => window.requests.filter(r => r.command === c).length, command); }
async function open(browser, width, options) {
  const page = await browser.newPage({ viewport: { width, height: 900 } });
  page.setDefaultTimeout(4000);
  page.errors = []; page.networkErrors = [];page.consoleErrors=[];
  page.on('pageerror', e => page.errors.push(String(e)));
  page.on('console', message=>{if(message.type()==='error')page.consoleErrors.push(message.text());});
  page.on('response', r => { if (r.status() >= 400) page.networkErrors.push(`${r.status()} ${r.url()}`); });
  if(options?.holdFonts) {
    page.fontRoutes=[];
    page.fontReady=new Promise(resolve=>{page.fontReadyResolve=resolve;});
    await page.route('**/fonts/*.otf',route=>{page.fontRoutes.push(route);if(page.fontRoutes.length===4)page.fontReadyResolve();});
  }
  await fixture(page, options);
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.getByRole('button', { name: '文档标签 synthetic.pdf', exact: true }).waitFor();
  return page;
}
async function activate(page) {
  await page.getByRole('button', { name: '批注', exact: true }).click();
  await page.getByRole('button', { name: '工具 · 文字批注', exact: true }).click();
  assert.equal(await page.locator('.inspector:visible').count(), 0, 'Text must not open old inspector');
  await page.waitForFunction(() => window.requests.some(r => r.command === 'free_texts_document') && document.querySelector('#pdf-page-image')?.complete);
}
async function start(page) {
  await activate(page);
  const box = await page.locator('#pdf-page-image').boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
}
async function body(page) { const editor = page.getByRole('textbox', { name: '页内文字正文', exact: true }); await editor.waitFor(); return editor; }
async function release(page, command, reply) {
  await page.evaluate(({ command, reply }) => {
    const pending = window.fixture.pending.find(p => !p.settled && p.command === command);
    if (!pending) throw new Error(`No pending ${command}`);
    pending.settled = true;
    if (reply?.reject) pending.reject(reply.reject);
    else {
      const value = reply?.value ?? pending.value;
      if (command === 'commit_free_text_editor') window.fixture.owner = value.free_text_editor || null;
      pending.resolve(value);
    }
  }, { command, reply });
}
async function waitCount(page, command, expected) {
  await page.waitForFunction(({ command, expected }) => window.requests.filter(r => r.command === command).length >= expected, { command, expected });
}
const appearance = { font: 'sans', font_size: 18, color: { r: 0, g: 0, b: 0 }, alignment: 'left', line_height: 1.3 };
const existing = { page_index: 0, annotation_index: 3, page_id: [9,0], object_id: [21,0], bounds: { x: 250, y: 370, width: 130, height: 95 }, contents: '原有文字 🦀', style: { font_resource: [70,49], font_size: 18, color: { r: 0, g: 0, b: 0 }, alignment: 'left' }, flags: 4, revision: Array(32).fill(19) };
const scenarios = [
  ['page-prompt-first-intent',{commit:'hold'},async page=>{
    await start(page);const editor=await body(page);await editor.fill('finish before native page prompt');
    await page.getByLabel('更多工具',{exact:true}).click();await page.getByRole('button',{name:'页面操作',exact:true}).click();
    await page.getByRole('button',{name:'提取页面…',exact:true}).click();await waitCount(page,'commit_free_text_editor',1);
    assert.equal(await count(page,'prompt_page_action'),0,'native input waits for Text handoff');
    await release(page,'commit_free_text_editor');await waitCount(page,'prompt_page_action',1);
    assert.deepEqual(await page.evaluate(()=>window.requests.find(r=>r.command==='prompt_page_action').args),{operation:'extract',page:0});
    assert.equal(await count(page,'commit_free_text_editor'),1);assert.equal(await page.locator('.inspector').count(),0);
  }],
  ...[['便笺', '便笺'], ['画笔', '手绘'], ['文字', '文字批注']].map(([entry, tool]) => [`sidebar-text-intent-${tool}`, { commit: 'hold' }, async page => {
    if (page.viewportSize().width <= 720) await page.getByRole('button', { name: '侧栏', exact: true }).click();
    await page.getByRole('button', { name: '侧栏批注', exact: true }).click();
    const button = page.getByRole('button', { name: `批注工具 · ${entry}`, exact: true });
    await button.waitFor();
    await start(page);
    await (await body(page)).fill('从批注侧栏完成文字并切换工具');
    assert.equal(await button.isEnabled(), true, 'actual accessible sidebar intent must remain enabled during text editing');
    await button.click(); await waitCount(page, 'commit_free_text_editor', 1);
    assert.equal(await page.getByRole('button', { name: '工具 · 文字批注', exact: true }).getAttribute('aria-pressed'), 'true', 'no tool switch before commit success');
    await button.click();
    assert.equal(await count(page, 'commit_free_text_editor'), 1, 'repeated pending sidebar intent cannot duplicate commit');
    await release(page, 'commit_free_text_editor');
    await page.waitForFunction(() => !document.querySelector('#canvas-text-body'));
    assert.equal(await count(page, 'commit_free_text_editor'), 1);
    assert.equal(await page.evaluate(() => window.fixture.writes), 1);
    assert.equal(await page.getByRole('button', { name: `工具 · ${tool}`, exact: true }).getAttribute('aria-pressed'), tool === '文字批注' ? 'false' : 'true', 'captured tool activates/toggles exactly once after commit');
    assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'), 'PDF 第 1 页');
  }]),
  ...['utf8', 'layout'].map(failure => [`local-${failure}-rejection-discards-failed-intent`, {}, async page => {
    await start(page);
    const editor = await body(page);
    if (failure === 'utf8') {
      const size = page.getByRole('spinbutton', { name: '文字字号', exact: true });
      await size.fill('1'); await size.dispatchEvent('change');
    }
    const rejected = failure === 'utf8' ? '汉'.repeat(21846) : Array(40).fill('完整保留的正文').join('\n');
    await editor.fill(rejected);
    await page.getByRole('button', { name: '下一页', exact: true }).click();
    assert.equal(await count(page, 'commit_free_text_editor'), 0, 'local rejection sends no mutation');
    assert.equal(await editor.inputValue(), rejected, 'rejected draft remains editable and complete');
    assert.ok((await page.locator('.canvas-text-recovery').innerText()).includes(failure === 'utf8' ? '64 KiB' : '正文超出'));
    assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'), 'PDF 第 1 页');
    const corrected = failure === 'utf8' ? '汉'.repeat(21845) : '已修正正文';
    await editor.fill(corrected);
    await page.getByRole('button', { name: '工具 · 便笺', exact: true }).click();
    await page.waitForFunction(() => !document.querySelector('#canvas-text-body'));
    assert.equal(await count(page, 'commit_free_text_editor'), 1, 'corrected draft commits exactly once');
    assert.equal(await page.evaluate(() => window.fixture.writes), 1);
    assert.equal(await page.evaluate(() => window.requests.find(r => r.command === 'commit_free_text_editor').args.contents), corrected);
    assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'), 'PDF 第 1 页', 'failed Next Page must not hijack corrected Note action');
    assert.equal(await page.getByRole('button', { name: '工具 · 便笺', exact: true }).getAttribute('aria-pressed'), 'true', 'latest explicit Note intent activates');
  }]),
  ['known-rejection-next-intent-replaces-failed-action',{commit:'reject'},async page=>{
    await start(page);await (await body(page)).fill('known refusal can be corrected');await page.getByRole('button',{name:'下一页',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.canvas-text-recovery')?.textContent.includes('core refusal'));
    await page.evaluate(()=>{window.fixture.options.commit=undefined;});await page.getByRole('button',{name:'工具 · 便笺',exact:true}).click();await page.waitForFunction(()=>document.querySelector('[aria-label="工具 · 便笺"]')?.getAttribute('aria-pressed')==='true');assert.equal(await count(page,'commit_free_text_editor'),2);assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'),'PDF 第 1 页','failed page action does not hijack next explicit tool intent');
  }],
  ['recovery-sync-focus-then-outside-finishes',{},async page=>{
    await start(page);await (await body(page)).fill('finish after recovery focus');await page.getByRole('button',{name:'同步文字状态',exact:true}).click();await waitCount(page,'document_info',2);assert.equal(await count(page,'commit_free_text_editor'),0);assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')),'同步文字状态');
    const reader=await page.locator('#pdf-reader').boundingBox();await page.mouse.click(reader.x+3,reader.y+3);await waitCount(page,'commit_free_text_editor',1);await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));assert.equal(await count(page,'commit_free_text_editor'),1);
  }],
  ...['reject','malformed','timeout'].map(mode=>[`post-handoff-refresh-${mode}-releases-gate`,{postCommitList:mode},async page=>{
    await start(page);await (await body(page)).fill('committed before read-only failure');await page.getByRole('button',{name:'工具 · 便笺',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));
    await page.waitForFunction(()=>document.body.textContent.includes('列表同步失败'));
    assert.equal(await page.getByRole('button',{name:'编辑',exact:true}).isEnabled(),true,'terminal read-only error releases its busy gate');assert.equal(await page.getByRole('button',{name:'工具 · 便笺',exact:true}).getAttribute('aria-pressed'),'false','failed refresh cannot dispatch captured intent');assert.equal(await count(page,'commit_free_text_editor'),1);
    await page.getByRole('button',{name:'编辑',exact:true}).click();assert.equal(await page.getByRole('button',{name:'编辑',exact:true}).getAttribute('aria-pressed'),'true');assert.equal(await count(page,'commit_free_text_editor'),1);
  }]),
  ['post-handoff-refresh-keeps-intent-gate',{postCommitList:'hold'},async page=>{
    await start(page);const editor=await body(page);await editor.fill('one captured intent');await page.getByRole('button',{name:'工具 · 便笺',exact:true}).click();await page.waitForFunction(()=>window.fixture.pending.some(p=>p.command==='free_texts_document'&&!p.settled));
    assert.equal(await editor.count(),0,'decoded normal page has no overlay during projection refresh');assert.equal(await page.getByRole('button',{name:'编辑',exact:true}).isDisabled(),true,'no newer action may enter before original intent refresh finishes');
    await release(page,'free_texts_document');await page.waitForFunction(()=>document.querySelector('[aria-label="工具 · 便笺"]')?.getAttribute('aria-pressed')==='true');assert.equal(await count(page,'commit_free_text_editor'),1);
  }],
  ['formatting-then-outside-finishes-without-refocus',{},async page=>{
    await start(page);const editor=await body(page);await editor.fill('format then outside');const font=page.getByRole('combobox',{name:'文字字体',exact:true});await font.focus();await font.selectOption('cjk_sc');assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')),'文字字体');assert.equal(await count(page,'commit_free_text_editor'),0);const reader=await page.locator('#pdf-reader').boundingBox();await page.mouse.click(reader.x+3,reader.y+3);await waitCount(page,'commit_free_text_editor',1);await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));
  }],
  ['outside-blur-finishes-once',{},async page=>{
    await start(page);const editor=await body(page);await editor.fill('outside finish');const reader=await page.locator('#pdf-reader').boundingBox();await page.mouse.click(reader.x+3,reader.y+3);await waitCount(page,'commit_free_text_editor',1);await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));assert.equal(await count(page,'commit_free_text_editor'),1);
  }],
  ['invalid-background-decode-keeps-page-owner-then-retry',{existing,appearance,background:'invalid-image'},async page=>{
    const before=await page.locator('#pdf-page-image').getAttribute('src');await start(page);await page.waitForFunction(()=>document.querySelector('.canvas-text-recovery')?.textContent.includes('解码失败'));assert.equal(await page.locator('#canvas-text-body').count(),0);assert.equal(await page.locator('#pdf-page-image').getAttribute('src'),before);
    await page.evaluate(()=>{window.fixture.options.background=undefined;});await page.getByRole('button',{name:'重试文字背景',exact:true}).click();await body(page);assert.equal(await count(page,'begin_canvas_free_text_editor'),1);assert.equal(await count(page,'render_free_text_editor_background'),2);
  }],
  ['old-staged-error-cannot-pollute-retried-editor',{existing,appearance,background:'invalid-image'},async page=>{
    await start(page);await page.waitForFunction(()=>document.querySelector('.canvas-text-recovery')?.textContent.includes('解码失败'));
    await page.evaluate(()=>{window.oldStaged=document.querySelector('.canvas-text-staged');window.fixture.options.background=undefined;});await page.getByRole('button',{name:'重试文字背景',exact:true}).click();const editor=await body(page);await editor.fill('retry remains editable');
    await page.evaluate(()=>window.oldStaged.dispatchEvent(new Event('error')));assert.equal((await page.locator('.canvas-text-recovery').innerText()).includes('解码失败'),false,'old staged error is not an error in current decoded editor');assert.equal(await editor.inputValue(),'retry remains editable');assert.equal(await count(page,'commit_free_text_editor'),0);
  }],
  ['same-png-retry-rejects-old-stage-timeout',{existing,appearance,background:'invalid-image',stageTimers:true},async page=>{
    await start(page);await page.waitForFunction(()=>document.querySelector('.canvas-text-recovery')?.textContent.includes('解码失败'));await page.evaluate(()=>{window.oldStageTimeout=[...window.fixture.timers.values()][0];if(!window.oldStageTimeout)throw Error('missing production stage timer');});
    await page.getByRole('button',{name:'重试文字背景',exact:true}).click();await waitCount(page,'render_free_text_editor_background',2);await page.waitForFunction(()=>window.fixture.timers.size>=2);
    const currentStatus=await page.locator('.canvas-text-recovery').innerText();await page.evaluate(()=>window.oldStageTimeout());assert.equal(await page.locator('.canvas-text-recovery').innerText(),currentStatus,'old attempt timer cannot change same-PNG new attempt result');assert.equal(await count(page,'commit_free_text_editor'),0);
    await page.evaluate(()=>[...window.fixture.timers.values()].at(-1)());assert.ok((await page.locator('.canvas-text-recovery').innerText()).includes('解码超时'),'current attempt timeout remains effective');
  }],
  ['cancelling-phase-rejects-stage-timeout',{existing,appearance,background:'invalid-image',stageTimers:true,cancel:'hold'},async page=>{
    await start(page);await page.waitForFunction(()=>document.querySelector('.canvas-text-recovery')?.textContent.includes('解码失败'));await page.evaluate(()=>{window.oldStageTimeout=[...window.fixture.timers.values()][0];});await page.getByRole('button',{name:'取消／同步文字',exact:true}).click();await waitCount(page,'cancel_free_text_editor',1);
    const cancelling=await page.locator('.canvas-text-recovery').innerText();await page.evaluate(()=>window.oldStageTimeout());assert.equal(await page.locator('.canvas-text-recovery').innerText(),cancelling,'decode timeout cannot overwrite cancellation phase');await release(page,'cancel_free_text_editor');await page.waitForFunction(()=>!document.querySelector('.canvas-text-recovery'));
  }],
  ['terminal-render-delay-keeps-protected-editor-preview',{terminalRender:'hold'},async page=>{
    await start(page);const editor=await body(page);await editor.fill('terminal handoff');const source=await page.locator('#pdf-page-image').getAttribute('src');await editor.press('Control+Enter');await page.waitForFunction(()=>window.fixture.pending.some(p=>p.command==='render_page'&&!p.settled));assert.equal(await editor.count(),1);assert.equal(await editor.getAttribute('readonly'),'');assert.equal(await page.locator('#pdf-page-image').getAttribute('src'),source);await release(page,'render_page');await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));assert.equal(await count(page,'commit_free_text_editor'),1);
  }],
  ...['doc','none','owner'].map(conflict=>[`read-only-sync-conflict-${conflict}`,{syncConflict:conflict},async page=>{
    await start(page);const editor=await body(page);await editor.fill('local body must stay');const source=await page.locator('#pdf-page-image').getAttribute('src');await page.getByRole('button',{name:'同步文字状态',exact:true}).click();await waitCount(page,'document_info',2);await page.waitForFunction(()=>document.querySelector('.canvas-text-recovery')?.textContent.includes('只读状态同步冲突'));
    assert.equal(await editor.inputValue(),'local body must stay');assert.equal(await page.locator('#pdf-page-image').getAttribute('src'),source);assert.equal(await count(page,'commit_free_text_editor'),0);assert.equal(await page.getByRole('button',{name:'下一页',exact:true}).isDisabled(),true);assert.equal(await page.getByRole('button',{name:'精确重试文字',exact:true}).isDisabled(),true);
    await page.getByRole('button',{name:'取消／同步文字',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('.canvas-text-recovery'));
  }]),
  ['old-read-only-sync-cannot-replace-new-owner',{sync:'hold'},async page=>{
    await start(page);await body(page);await page.getByRole('button',{name:'同步文字状态',exact:true}).click();await waitCount(page,'document_info',2);await page.getByRole('button',{name:'取消／同步文字',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('.canvas-text-recovery'));
    await start(page);const editor=await body(page);await editor.fill('new owner local body');const source=await page.locator('#pdf-page-image').getAttribute('src');await release(page,'document_info');assert.equal(await editor.inputValue(),'new owner local body');assert.equal(await page.locator('#pdf-page-image').getAttribute('src'),source);assert.equal(await count(page,'commit_free_text_editor'),0);assert.equal(await count(page,'begin_canvas_free_text_editor'),2);
  }],
  ['ime-final-input-after-compositionend-is-not-lost',{commit:'hold'},async page=>{
    await start(page);const editor=await body(page);await editor.fill('旧组合');await editor.dispatchEvent('compositionstart');await page.getByRole('button',{name:'工具 · 便笺',exact:true}).click();assert.equal(await count(page,'commit_free_text_editor'),0);
    await editor.evaluate(n=>{n.value='最终完整正文';n.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'最终完整正文'}));n.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertFromComposition',data:'最终完整正文'}));});
    await waitCount(page,'commit_free_text_editor',1);const sent=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_free_text_editor').args);assert.equal(sent.contents,'最终完整正文','deferred IME commit must include final input event after compositionend');
  }],
  ['same-owner-read-only-sync-preserves-local-body-selection-background',{sync:'hold'},async page=>{
    await start(page);const editor=await body(page);await editor.fill('typed original local');await editor.evaluate(n=>n.setSelectionRange(2,7));const source=await page.locator('#pdf-page-image').getAttribute('src');const begin=await count(page,'begin_canvas_free_text_editor');
    const sync=page.getByRole('button',{name:'同步文字状态',exact:true});assert.equal(await sync.count(),1,'real read-only recovery sync must be reachable');await sync.click();await waitCount(page,'document_info',2);
    await editor.fill('newer typed local survives');await editor.evaluate(n=>n.setSelectionRange(1,6));await release(page,'document_info');
    await page.waitForFunction(()=>!document.querySelector('[aria-label="同步文字状态"]')?.disabled);
    assert.equal(await editor.inputValue(),'newer typed local survives');assert.equal(await editor.evaluate(n=>n.selectionStart),1);assert.equal(await editor.evaluate(n=>n.selectionEnd),6);assert.equal(await page.locator('#pdf-page-image').getAttribute('src'),source);assert.equal(await count(page,'commit_free_text_editor'),0);assert.equal(await count(page,'begin_canvas_free_text_editor'),begin);
  }],
  ...['move','resize'].map(operation=>[`overflow-recover-only-${operation}`,{},async page=>{
    await activate(page);await page.locator('#pdf-reader').evaluate(n=>{n.scrollTop=n.scrollHeight;});const box=await page.locator('#pdf-page-image').boundingBox();await page.mouse.click(box.x+box.width*.5,box.y+box.height*(1-60/842));
    const editor=await body(page);const contents=Array(8).fill('正文完整保留').join('\n');await editor.fill(contents);
    assert.ok((await page.locator('.canvas-text-recovery').innerText()).includes('正文超出'));
    const handle=page.getByRole('button',{name:operation==='move'?'移动文字框':'调整文字框 n',exact:true});const h=await handle.boundingBox();await page.mouse.move(h.x+h.width*.25,h.y+h.height*.25);await page.mouse.down();await page.mouse.move(h.x+h.width*.25,h.y+h.height*.25-230);await page.mouse.up();
    assert.equal(await editor.inputValue(),contents);
    await page.getByRole('button',{name:'完成文字',exact:true}).click();await waitCount(page,'commit_free_text_editor',1);await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));
    const payload=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_free_text_editor').args);assert.equal(payload.contents,contents);assert.ok(payload.bounds.height>=18*1.3*8-.1);
  }]),
  ['codeunit-overflow-not-silently-truncated',{},async page=>{
    await start(page);const editor=await body(page);const contents='A'.repeat(65537);await editor.fill(contents);
    assert.equal((await editor.inputValue()).length,contents.length,'local input beyond codeunit cap must not silently truncate');await editor.press('Control+Enter');assert.equal(await count(page,'commit_free_text_editor'),0);assert.ok((await page.locator('.canvas-text-recovery').innerText()).includes('64 KiB'));
  }],
  ['pointer-drag-secondary-stale-image-frame',{},async page=>{
    await activate(page);const box=await page.locator('#pdf-page-image').boundingBox();const p={x:box.x+box.width*.5,y:box.y+box.height*.5};
    await page.mouse.move(p.x,p.y);await page.mouse.down();await page.mouse.move(p.x+8,p.y);await page.mouse.move(p.x,p.y);await page.mouse.up();assert.equal(await count(page,'begin_canvas_free_text_editor'),0,'drag out and back is not a click');
    await page.mouse.click(p.x,p.y,{button:'right'});assert.equal(await count(page,'begin_canvas_free_text_editor'),0,'secondary pointer is not begin');
    await page.mouse.move(p.x,p.y);await page.mouse.down();await page.locator('#pdf-page-image').evaluate(n=>{n.style.transform='scale(.98)';});await page.mouse.up();await page.locator('#pdf-page-image').evaluate(n=>{n.style.transform='';});assert.equal(await count(page,'begin_canvas_free_text_editor'),0,'stale decoded frame cannot begin');
    for(const event of ['pointercancel','lostpointercapture']){await page.mouse.move(p.x,p.y);await page.mouse.down();await page.locator('.drawing-surface').dispatchEvent(event,{pointerId:1,isPrimary:true,button:0,clientX:p.x,clientY:p.y});await page.mouse.up();assert.equal(await count(page,'begin_canvas_free_text_editor'),0,event);}
  }],
  ['utf8-overflow-retained-then-one-character-admitted',{},async page=>{
    await start(page);const editor=await body(page);
    await page.getByRole('spinbutton',{name:'文字字号',exact:true}).fill('1');await page.getByRole('spinbutton',{name:'文字字号',exact:true}).dispatchEvent('change');
    await editor.fill('汉'.repeat(21846));await editor.press('Control+Enter');
    assert.equal(await count(page,'commit_free_text_editor'),0);assert.equal((await editor.inputValue()).length,21846);
    assert.ok((await page.locator('.canvas-text-recovery').innerText()).includes('64 KiB'));
    await editor.fill('汉'.repeat(21845));await editor.press('Control+Enter');await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));
    assert.equal(await count(page,'commit_free_text_editor'),1);
  }],
  ['late-background-and-old-image-do-not-overwrite-new-owner',{background:'hold'},async page=>{
    await start(page);await waitCount(page,'render_free_text_editor_background',1);
    await page.getByRole('button',{name:'取消／同步文字',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('.canvas-text-recovery'));
    await page.evaluate(()=>{window.fixture.options.background=undefined;window.oldImage=document.querySelector('#pdf-page-image');});
    await start(page);const editor=await body(page);await editor.fill('new owner remains');
    const before=await page.locator('#pdf-page-image').getAttribute('src');
    await release(page,'render_free_text_editor_background');
    await page.evaluate(()=>{window.oldImage.dispatchEvent(new Event('load'));window.oldImage.dispatchEvent(new Event('error'));});
    assert.equal(await editor.inputValue(),'new owner remains');assert.equal(await page.locator('#pdf-page-image').getAttribute('src'),before);assert.equal(await count(page,'begin_canvas_free_text_editor'),2);
  }],
  ['cancel-malformed-confirmation-blocks-new-owner',{begin:'hold',cancel:'malformed'},async page=>{
    await start(page);await waitCount(page,'begin_canvas_free_text_editor',1);await page.getByRole('button',{name:'取消／同步文字',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('.canvas-text-recovery')?.textContent.includes('响应格式错误'));
    assert.equal(await page.getByRole('button',{name:'工具 · 便笺',exact:true}).isDisabled(),true);
    await release(page,'begin_canvas_free_text_editor');assert.equal(await page.locator('#canvas-text-body').count(),0);
    await page.evaluate(()=>{window.fixture.options.cancel=undefined;});await page.getByRole('button',{name:'取消／同步文字',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('.canvas-text-recovery'));
    assert.equal(await count(page,'cancel_canvas_free_text_request'),2);
  }],
  ['pointer-old-tool-must-not-begin', {}, async page=>{
    await activate(page);const box=await page.locator('#pdf-page-image').boundingBox();const point={x:box.x+box.width/2,y:box.y+box.height/2};
    await page.mouse.move(point.x,point.y);await page.mouse.down();
    await page.getByRole('button',{name:'工具 · 便笺',exact:true}).evaluate(n=>n.click());
    await page.mouse.up();
    assert.equal(await count(page,'begin_canvas_free_text_editor'),0,'old Text pointer cannot begin after changing active tool');
  }],
  ...['mode','page','tab','route','zoom','fit','close','export','sidebar-page','overview'].map(action => [`intent-${action}`, { commit: 'hold' }, async page => {
    if(action==='sidebar-page'&&page.viewportSize().width<=720) await page.getByRole('button',{name:'侧栏',exact:true}).click();
    await start(page); const editor=await body(page); await editor.fill(`captured ${action}`);
    const originalWidth = (await page.locator('#pdf-page-image').boundingBox()).width;
    const mainRenders = () => page.evaluate(()=>window.requests.filter(r=>r.command==='render_page'&&r.args.scale>.25).length);
    const initialRenders = await mainRenders();
    const trigger = {
      mode: () => page.getByRole('button',{name:'编辑',exact:true}).click(),
      page: () => page.getByRole('button',{name:'下一页',exact:true}).click(),
      tab: () => page.getByRole('button',{name:'文档标签 second.pdf',exact:true}).click(),
      route: () => page.getByRole('button',{name:'主界面标签',exact:true}).click(),
      zoom: () => page.getByRole('combobox',{name:'缩放',exact:true}).selectOption('1.5'),
      fit: () => page.getByRole('button',{name:'适合页面',exact:true}).click(),
      close: () => page.getByRole('button',{name:'关闭 second.pdf',exact:true}).click(),
      export: () => page.getByRole('button',{name:'导出',exact:true}).click(),
      'sidebar-page': () => page.getByRole('button',{name:'第 2 页',exact:true}).click(),
      overview: async () => {const button=page.getByRole('button',{name:'页面预览',exact:true});assert.equal(await button.isEnabled(),true,'overview mode intent must remain enabled');await button.click();},
    }[action];
    await trigger(); await waitCount(page,'commit_free_text_editor',1);
    assert.equal(await editor.count(),1,'editor stays through pending commit');
    assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'),'PDF 第 1 页','original page stays before commit success');
    assert.equal(await count(page,'activate_document'),0);assert.equal(await count(page,'close_document'),0);
    await release(page,'commit_free_text_editor');
    await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));
    if(action==='mode') await page.waitForFunction(()=>document.querySelector('.chrome-mode.active')?.textContent==='编辑');
    if(action==='page'||action==='sidebar-page') await page.waitForFunction(()=>document.querySelector('#pdf-page-image')?.alt==='PDF 第 2 页');
    if(action==='tab') {await waitCount(page,'activate_document',1);assert.equal(await count(page,'activate_document'),1);}
    if(action==='route') assert.equal(await page.locator('.home-host').isVisible(),true);
    if(action==='zoom') {await page.waitForFunction(()=>document.querySelector('.chrome-zoom')?.textContent.includes('150%'));assert.ok((await page.locator('#pdf-page-image').boundingBox()).width>originalWidth);}
    if(action==='fit') {await page.waitForFunction(n=>window.requests.filter(r=>r.command==='render_page'&&r.args.scale>.25).length>=n,initialRenders+2);assert.equal(await mainRenders(),initialRenders+2,'one commit handoff and one captured fit render (excluding thumbnail renders)');}
    if(action==='close') {await waitCount(page,'close_document',1);assert.equal(await count(page,'close_document'),1);}
    if(action==='export') {await page.getByRole('button',{name:'工具 · 存储副本',exact:true}).click();await waitCount(page,'export_document',1);assert.equal(await count(page,'export_document'),1);}
    if(action==='overview') await page.waitForFunction(()=>!document.querySelector('.page-overview')?.hidden);
    assert.equal(await count(page,'commit_free_text_editor'),1);
  }]),
  ['font-admission-waits-for-four-packaged-fonts', { holdFonts: true }, async page => {
    await start(page); await page.fontReady;
    assert.equal(await page.locator('#canvas-text-body').count(),0,'fonts must finish loading before admitted editor input');
    await Promise.all(page.fontRoutes.map(route=>route.continue()));
    const editor=await body(page);assert.equal(await editor.inputValue(),'');
  }],
  ['list-failure-is-not-blank-page', { listError:true }, async page => {
    await start(page);
    assert.equal(await count(page,'begin_canvas_free_text_editor'),0);
    assert.ok((await page.locator('.statusbar').innerText()).includes('未把未知结果当作空白页'));
  }],
  ['unsupported-hit-preserves-original', { unsupported:{page_index:0,annotation_index:5,page_id:[9,0],object_id:[22,0],subtype:'FreeText',contents:'unsupported',color:null,bounds:{x:250,y:370,width:130,height:95},modified:null} }, async page => {
    await start(page);
    assert.equal(await count(page,'begin_canvas_free_text_editor'),0);
    assert.ok((await page.locator('.statusbar').innerText()).includes('原件已保留'));
  }],
  ['background-error-retains-owner-normal-page', { existing,appearance,background:'reject' }, async page => {
    const original=await page.locator('#pdf-page-image').getAttribute('src');
    await start(page);await waitCount(page,'render_free_text_editor_background',1);
    await page.waitForFunction(()=>document.querySelector('.canvas-text-recovery')?.textContent.includes('编辑背景未就绪'));
    assert.equal(await page.locator('#canvas-text-body').count(),0);assert.equal(await page.locator('#pdf-page-image').getAttribute('src'),original);
    await page.getByRole('button',{name:'取消／同步文字',exact:true}).click();await waitCount(page,'cancel_free_text_editor',1);
    await page.waitForFunction(()=>!document.querySelector('.canvas-text-recovery'));
  }],
  ['malformed-document-keeps-frozen-retry', { commit:'malformed' }, async page => {
    await start(page);const editor=await body(page);await editor.fill('精确冻结');await editor.press('Control+Enter');
    await page.getByRole('button',{name:'精确重试文字',exact:true}).waitFor();
    assert.equal(await editor.inputValue(),'精确冻结');assert.equal(await editor.getAttribute('readonly'),'');
    await page.evaluate(()=>{window.fixture.options.commit=undefined;});
    await page.getByRole('button',{name:'精确重试文字',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));
    const payloads=await page.evaluate(()=>window.requests.filter(r=>r.command==='commit_free_text_editor').map(r=>r.args));assert.equal(payloads.length,2);assert.deepEqual(payloads[0],payloads[1]);
  }],
  ...[64,128,512].map(flags=>[`imported-lock-${flags}`, {existing:{...existing,flags},appearance},async page=>{
    await start(page);const editor=await body(page);
    assert.equal((await editor.getAttribute('readonly'))!==null,flags!==128);
    assert.equal(await page.getByRole('combobox',{name:'文字字体',exact:true}).isDisabled(),flags!==512);
    if(flags===128) {await editor.fill('正文可改');await editor.press('Control+Enter');await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));const sent=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_free_text_editor').args);assert.deepEqual(sent.bounds,existing.bounds);}
    else {const before=await page.locator('.canvas-text-editor').getAttribute('style');const h=await page.getByRole('button',{name:'移动文字框',exact:true}).boundingBox();await page.mouse.move(h.x+h.width*.25,h.y+h.height*.25);await page.mouse.down();await page.mouse.move(h.x+h.width*.25+15,h.y+h.height*.25+15);await page.mouse.up();const after=await page.locator('.canvas-text-editor').getAttribute('style');assert.equal(after===before,flags===64);}
  }]),
  ['unknown-style-none-pure-move-explicit-font-replacement', {existing,appearance,unknownStyle:true},async page=>{
    await start(page);const editor=await body(page);assert.equal(await editor.getAttribute('readonly'),'');
    const h=await page.getByRole('button',{name:'移动文字框',exact:true}).boundingBox();await page.mouse.move(h.x+h.width*.25,h.y+h.height*.25);await page.mouse.down();await page.mouse.move(h.x+h.width*.25+10,h.y+h.height*.25);await page.mouse.up();
    await page.getByRole('button',{name:'完成文字',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));
    const sent=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_free_text_editor').args);assert.equal(sent.style,null);assert.equal(sent.contents,existing.contents);assert.notEqual(sent.bounds.x,existing.bounds.x);
    await start(page);const again=await body(page);assert.equal(await again.getAttribute('readonly'),'');
    await page.getByRole('combobox',{name:'文字字体',exact:true}).selectOption('cjk_sc');assert.equal(await again.getAttribute('readonly'),null);await again.fill('明确字体替换');
  }],
  ['unknown-style-null-exact-uncertain-retry', {existing,appearance,unknownStyle:true,commit:'timeout'},async page=>{
    await start(page);await body(page);await page.getByRole('button',{name:'完成文字',exact:true}).click();
    await page.getByRole('button',{name:'精确重试文字',exact:true}).waitFor();
    assert.equal(await page.getByRole('combobox',{name:'文字字体',exact:true}).isDisabled(),true);
    await page.evaluate(()=>{window.fixture.options.commit=undefined;});await page.getByRole('button',{name:'精确重试文字',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('#canvas-text-body'));
    const payloads=await page.evaluate(()=>window.requests.filter(r=>r.command==='commit_free_text_editor').map(r=>r.args));
    assert.equal(payloads.length,2);assert.equal(Object.hasOwn(payloads[0],'style'),true);assert.equal(payloads[0].style,null);assert.deepEqual(payloads[1],payloads[0]);assert.equal(await page.evaluate(()=>window.fixture.writes),1);
  }],
  ...[0,90,180,270].map(rotation=>[`actual-text-plane-rotation-${rotation}`,{rotation,existing,appearance},async page=>{
    await start(page);await body(page);
    const mapped=await page.locator('.canvas-text-editor').evaluate(node=>{const m=new DOMMatrix(getComputedStyle(node).transform);return {matrix:[m.a,m.b,m.c,m.d,m.e,m.f],width:parseFloat(node.style.width),height:parseFloat(node.style.height)};});
    const pageBox=await page.locator('#pdf-page-image').boundingBox();const [a,b,c,d,e,f]=mapped.matrix;
    const project=(x,y)=>rotation===0?[x,842-y]:rotation===90?[y,x]:rotation===180?[595-x,y]:[842-y,595-x];
    const dw=rotation%180?842:595,dh=rotation%180?595:842;
    for(const [lx,ly,x,y] of [[0,0,250,465],[130,0,380,465],[0,95,250,370]]){const [px,py]=project(x,y);assert.ok(Math.abs(a*lx+c*ly+e-px/dw*pageBox.width)<.05);assert.ok(Math.abs(b*lx+d*ly+f-py/dh*pageBox.height)<.05);}
    assert.ok(Math.abs(pageBox.width/pageBox.height-dw/dh)<.002,'decoded source must match real rotated aspect');
  }]),
  ['cold-owner-recovery-warning', { owner: { id: 99, request: 'fedcba9876543210fedcba9876543210', page: 0, target: { kind: 'existing', original: existing }, appearance_style: appearance }, existing, appearance }, async page => {
    const editor = await body(page); assert.equal(await editor.inputValue(), existing.contents);
    const status = await page.locator('.statusbar').innerText();
    assert.ok(status.includes('未提交输入') && status.includes('持久化'), `cold recovery must warn: ${status}`);
    assert.equal(await count(page, 'begin_canvas_free_text_editor'), 0, 'recovery consumes owner without duplicate begin');
  }],
  ['stable-body-fonts-formatting', {}, async page => {
    const positions = {};
    const geometry=()=>page.evaluate(()=>{const reader=document.querySelector('#pdf-reader'),img=document.querySelector('#pdf-page-image');const a=reader.getBoundingClientRect(),b=img.getBoundingClientRect();return {scrollTop:reader.scrollTop,scrollHeight:reader.scrollHeight,clientHeight:reader.clientHeight,readerTop:a.top,imageTop:b.top,imageHeight:b.height};});
    positions.before=await geometry();
    await start(page); const editor = await body(page);
    positions.editing=await geometry();
    await editor.evaluate(n => { window.originalBody = n; });
    const initial = await page.locator('.canvas-text-editor').evaluate(n => n.style.width);
    await editor.fill('A'); await editor.press('End'); await editor.type('BCDEFGHIJKLMNOPQRSTUVWXYZ');
    await editor.press('Enter'); await editor.type('第二行');
    assert.equal(await editor.inputValue(), 'ABCDEFGHIJKLMNOPQRSTUVWXYZ\n第二行');
    assert.equal(await editor.evaluate(n => n === window.originalBody), true, 'textarea node must not rebuild');
    assert.notEqual(await page.locator('.canvas-text-editor').evaluate(n => n.style.width), initial, 'real content changes geometry');
    const fonts = await page.evaluate(() => [...document.fonts].filter(f => f.family.startsWith('PM ')).map(f => [f.family, f.status]));
    assert.equal(fonts.length, 4); assert.ok(fonts.every(([,status]) => status === 'loaded'), JSON.stringify(fonts));
    await page.getByRole('combobox', { name: '文字字体', exact: true }).selectOption('cjk_sc');
    assert.equal(await count(page, 'commit_free_text_editor'), 0, 'formatting focus is inside transaction');
    await editor.click(); await editor.evaluate(n => n.setSelectionRange(1, 5));
    assert.equal(await editor.evaluate(n => n.selectionEnd - n.selectionStart), 4);
    await page.locator('#pdf-reader').evaluate(n=>{n.scrollTop=0;});
    positions.top=await geometry();
    assert.ok(positions.top.imageTop>=positions.top.readerTop,'page top must remain scroll-reachable');
    await page.locator('#pdf-reader').evaluate(n=>{n.scrollTop=n.scrollHeight;});
    positions.bottom=await geometry();
    assert.ok(positions.bottom.imageTop+positions.bottom.imageHeight<=positions.bottom.readerTop+positions.bottom.clientHeight+1,'page bottom must remain scroll-reachable');
    fs.writeFileSync(path.join(evidence,`geometry-${page.viewportSize().width}.json`),JSON.stringify(positions,null,2));
    await editor.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(evidence, `editing-${page.viewportSize().width}.png`), fullPage: true });
    await editor.press('Control+Enter');
    await page.waitForFunction(() => !document.querySelector('#canvas-text-body'));
    assert.equal(await count(page, 'commit_free_text_editor'), 1);
  }],
  ['manual-resize-preserves-opposite-corner', {}, async page => {
    await start(page); const editor = await body(page); await editor.fill('size');
    const before = await page.locator('.canvas-text-editor').evaluate(n => ({ height: parseFloat(n.style.height), width: parseFloat(n.style.width), transform: n.style.transform }));
    const handle = await page.getByRole('button', { name: '调整文字框 se', exact: true }).boundingBox();
    await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2); await page.mouse.down(); await page.mouse.move(handle.x + 50, handle.y + 60); await page.mouse.up();
    const after = await page.locator('.canvas-text-editor').evaluate(n => ({ height: parseFloat(n.style.height), width: parseFloat(n.style.width), transform: n.style.transform }));
    assert.ok(after.height > before.height + 20, `manual resize height must survive pointerup: ${JSON.stringify({ before, after })}`);
    assert.ok(after.width > before.width);
    const matrix = value => value.match(/[-+]?\d*\.?\d+(?:e[-+]?\d+)?/g).map(Number);
    const a = matrix(after.transform), b = matrix(before.transform);
    assert.ok(Math.abs(a[4]-b[4]) < .01 && Math.abs(a[5]-b[5]) < .01, 'SE resize preserves opposite top-left corner');
    assert.equal(await count(page, 'commit_free_text_editor'), 0);
  }],
  ['ime-single-intent', { commit: 'hold' }, async page => {
    await start(page); const editor = await body(page); await editor.fill('中文组合');
    await editor.dispatchEvent('compositionstart');
    await page.getByRole('button', { name: '工具 · 便笺', exact: true }).click();
    assert.equal(await count(page, 'commit_free_text_editor'), 0, 'IME tool intent cannot prematurely submit');
    await editor.dispatchEvent('compositionend'); await waitCount(page, 'commit_free_text_editor', 1);
    await editor.dispatchEvent('blur'); await page.getByRole('button', { name: '工具 · 便笺', exact: true }).click();
    await editor.dispatchEvent('blur');
    assert.equal(await count(page, 'commit_free_text_editor'), 1);
    await release(page, 'commit_free_text_editor');
    await page.waitForFunction(() => document.querySelector('[aria-label="工具 · 便笺"]')?.getAttribute('aria-pressed') === 'true');
    assert.equal(await count(page, 'commit_free_text_editor'), 1);
  }],
  ['exact-retry-unknown-result', { commit: 'timeout' }, async page => {
    await start(page); const editor = await body(page); await editor.fill('冻结全部 payload 🦀');
    await editor.press('Control+Enter'); await page.getByRole('button', { name: '精确重试文字', exact: true }).waitFor();
    assert.equal(await editor.getAttribute('readonly'), '');
    await page.evaluate(() => { window.fixture.options.commit = undefined; });
    await page.getByRole('button', { name: '精确重试文字', exact: true }).click();
    await page.waitForFunction(() => !document.querySelector('#canvas-text-body'));
    const sent = await page.evaluate(() => window.requests.filter(r => r.command === 'commit_free_text_editor').map(r => r.args));
    assert.equal(sent.length, 2); assert.deepEqual(sent[1], sent[0]);
    assert.equal(await page.evaluate(() => window.fixture.writes), 1);
  }],
  ...['timeout', 'malformed'].map(commit => [`uncertain-${commit}-retains-original-action`, { commit }, async page => {
    await start(page); await (await body(page)).fill('冻结原动作与完整正文');
    await page.getByRole('button', { name: '下一页', exact: true }).click();
    await page.getByRole('button', { name: '精确重试文字', exact: true }).waitFor();
    assert.equal(await page.getByRole('button', { name: '工具 · 便笺', exact: true }).isEnabled(), false);
    assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'), 'PDF 第 1 页');
    await page.evaluate(() => { window.fixture.options.commit = undefined; });
    await page.getByRole('button', { name: '精确重试文字', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('#pdf-page-image')?.alt === 'PDF 第 2 页');
    const sent = await page.evaluate(() => window.requests.filter(r => r.command === 'commit_free_text_editor').map(r => r.args));
    assert.equal(sent.length, 2); assert.deepEqual(sent[1], sent[0]);
    assert.equal(await page.evaluate(() => window.fixture.writes), 1);
    assert.equal(await page.getByRole('button', { name: '工具 · 便笺', exact: true }).getAttribute('aria-pressed'), 'false');
  }]),
  ['core-failure-keeps-context', { commit: 'reject' }, async page => {
    await start(page); const editor = await body(page); await editor.fill('正文不可丢');
    await page.getByRole('button', { name: '下一页', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.canvas-text-recovery')?.textContent.includes('core refusal'));
    assert.equal(await editor.inputValue(), '正文不可丢');
    assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'), 'PDF 第 1 页');
    assert.equal(await count(page, 'commit_free_text_editor'), 1);
  }],
  ['delayed-background-existing-identity', { existing, appearance, background: 'hold' }, async page => {
    await start(page); await waitCount(page, 'render_free_text_editor_background', 1);
    assert.equal(await page.locator('#canvas-text-body').count(), 0, 'not editable before protected background decode');
    await release(page, 'render_free_text_editor_background');
    const editor = await body(page); assert.equal(await editor.inputValue(), existing.contents);
    await editor.press('Escape'); await page.waitForFunction(() => !document.querySelector('#canvas-text-body'));
    assert.equal(await count(page, 'cancel_free_text_editor'), 1);
  }],
  ['late-begin-cancel-tombstone', { begin: 'hold', cancel: 'hold' }, async page => {
    await start(page); await waitCount(page, 'begin_canvas_free_text_editor', 1);
    await page.getByRole('button', { name: '取消／同步文字', exact: true }).click();
    await waitCount(page, 'cancel_canvas_free_text_request', 1);
    await release(page, 'begin_canvas_free_text_editor');
    assert.equal(await page.locator('#canvas-text-body').count(), 0);
    assert.equal(await count(page, 'render_free_text_editor_background'), 0);
    await release(page, 'cancel_canvas_free_text_request');
    await page.waitForFunction(() => !document.querySelector('.canvas-text-recovery'));
  }],
];
(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_EXECUTABLE_PATH });
  try {
    let passed = 0;
    for (const width of [1280, 390]) {
      if (!process.env.TEXT_CASE || process.env.TEXT_CASE === 'seamless-tool') {
      const page = await open(browser, width);
      await start(page);
      const editor = await body(page);
      assert.equal(await count(page, 'begin_canvas_free_text_editor'), 1);
      await editor.fill('页内实际输入');
      assert.equal(await page.getByRole('button', { name: '工具 · 便笺', exact: true }).isEnabled(), true, 'tool intent must remain clickable with active text');
      await page.getByRole('button', { name: '工具 · 便笺', exact: true }).click();
      await page.waitForFunction(() => document.querySelector('[aria-label="工具 · 便笺"]')?.getAttribute('aria-pressed') === 'true');
      assert.equal(await count(page, 'commit_free_text_editor'), 1, 'seamless tool action commits once');
      assert.equal(await editor.count(), 0);
      assert.deepEqual(page.errors, []);
      assert.deepEqual(page.consoleErrors, []);
      assert.deepEqual(page.networkErrors, []);
      await page.close();
      passed++;
      }
      for (const [name, options, run] of scenarios) {
        if (process.env.TEXT_CASE && process.env.TEXT_CASE !== name) continue;
        const page = await open(browser, width, options);
        try { await run(page); assert.deepEqual(page.errors, []); assert.deepEqual(page.consoleErrors, []); assert.deepEqual(page.networkErrors, []); passed++; }
        catch (error) { await page.screenshot({ path: path.join(evidence, `FAIL-${name}-${width}.png`), fullPage: true }); fs.writeFileSync(path.join(evidence, `FAIL-${name}-${width}.html`), await page.content()); console.error(`${name} @${width}; evidence ${evidence}`); throw error; }
        finally { await page.close(); }
      }
    }
    console.log(`canvas-text browser: ${passed} viewport cases passed; console/network errors 0; evidence ${evidence}`);
  } finally { await browser.close(); server.close(); }
})().catch(error => { console.error(error); server.close(); process.exitCode = 1; });
