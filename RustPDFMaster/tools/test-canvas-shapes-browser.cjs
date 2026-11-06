// Compiled Rust/WASM with controlled synthetic IPC. Not native or Windows QA.
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../dist');
const evidence = fs.mkdtempSync(path.join(os.tmpdir(), 'pdfmaster-canvas-shapes-ui-'));
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
  await page.addInitScript(options => {
    window.requests = [];
    const f = window.fixture = { options, owner: options.owner || null, pending: [], next: 10, activeId: 7, writes: 0, committed: {}, selections: options.selections || [], revision: Array(32).fill(17) };
    if(options.stageTimers){const schedule=window.setTimeout.bind(window),clear=window.clearTimeout.bind(window);f.timers=new Map();window.setTimeout=(callback,delay,...args)=>{const id=schedule(callback,delay,...args);if(delay===15000)f.timers.set(id,callback);return id;};window.clearTimeout=id=>{f.timers.delete(id);return clear(id);};}
    const raster = omission => {
      const canvas = document.createElement('canvas');
      canvas.width = options.rotation % 180 ? 842 : 595;
      canvas.height = options.rotation % 180 ? 595 : 842;
      const c = canvas.getContext('2d');
      c.fillStyle = '#edf4f8'; c.fillRect(0, 0, canvas.width, canvas.height);
      c.fillStyle = '#c7e1ed'; c.fillRect(0, 0, canvas.width, 78);
      c.fillStyle = '#e7cca2'; c.fillRect(0, canvas.height - 112, canvas.width, 112);
      c.fillStyle = omission ? '#468297' : '#485d6c'; c.font = '16px sans-serif';
      c.fillText(`Synthetic Shapes IPC ${omission ? 'omission' : 'normal'} · ${f.writes} · text ${f.textWrites||0}`, 25, 45);
      // A controlled raster producer, not a real PDF/AP renderer. Render saved
      // geometry too so terminal screenshots are not merely blue selection boxes.
      c.save();
      const transforms={0:[1,0,0,-1,0,842],90:[0,1,1,0,0,0],180:[-1,0,0,1,595,0],270:[0,-1,-1,0,842,595]};
      c.setTransform(...transforms[options.rotation||0]);
      const css=color=>`rgb(${color.r*255},${color.g*255},${color.b*255})`;
      for(const s of f.selections){
        if(omission&&f.owner?.target.kind==='existing'&&s.original.annotation_index===f.owner.target.original.original.annotation_index)continue;
        const a=s.original,b=a.bounds;c.save();c.beginPath();c.rect(b.x,b.y,b.width,b.height);c.clip();c.globalAlpha=s.opacity;c.strokeStyle=css(a.stroke);c.lineWidth=a.line_width;c.fillStyle=a.fill?css(a.fill):'transparent';
        c.beginPath();if(a.shape==='rectangle')c.rect(b.x+a.line_width/2,b.y+a.line_width/2,b.width-a.line_width,b.height-a.line_width);
        else if(a.shape==='ellipse')c.ellipse(b.x+b.width/2,b.y+b.height/2,(b.width-a.line_width)/2,(b.height-a.line_width)/2,0,0,Math.PI*2);
        else{c.setLineDash(a.line.dash_pattern);c.moveTo(a.line.start.x,a.line.start.y);c.lineTo(a.line.end.x,a.line.end.y);}
        if(a.fill)c.fill();c.stroke();c.restore();
      }c.restore();
      return canvas.toDataURL('image/png');
    };
    const view = () => ({ id: f.activeId, name: 'synthetic.pdf', tabs: [{ id: 7, name: 'synthetic.pdf', dirty: f.writes > 0 }, { id: 8, name: 'second.pdf', dirty: false }], restored: false, shape_editor: f.owner, free_text_editor:f.textOwner||options.textOwner||null, info: { page_count: 2, pages: [0,1].map(() => ({ width: 595, height: 842, rotation: options.rotation || 0 })), dirty: f.writes > 0, can_undo: f.writes > 0, can_redo: false } });
    const outcome = (command, args, value, mode) => {
      if (mode === 'hold') return new Promise((resolve, reject) => f.pending.push({ command, args, value, resolve, reject, settled: false }));
      if (mode === 'reject') throw 'core refusal: synthetic';
      if (mode === 'timeout') throw '原生响应超时；结果未知';
      if (mode === 'malformed') return { invalid: true };
      if (mode === 'invalid-image') return 'data:image/png;base64,AAAA';
      return value;
    };
    const canonical = v => JSON.stringify(v, (_k, x) => x && !Array.isArray(x) && typeof x === 'object' ? Object.fromEntries(Object.entries(x).sort(([a],[b]) => a.localeCompare(b))) : x);
    const projection = () => ({ annotations: f.selections, unsupported_count: options.rows?.length || 0, annotation_count: f.selections.length + (options.rows?.length || 0), revision: options.projectionRevision||f.revision,
      hit_rows: [...f.selections.map(s => ({ page_index: s.page_index, annotation_index: s.original.annotation_index, page_id: s.page_id, object_id: s.object_id, subtype: {rectangle:'Square',ellipse:'Circle',line:'Line'}[s.original.shape], contents: '', color: null, bounds: s.original.bounds, modified: null })), ...(options.rows || [])].sort((a,b)=>a.annotation_index-b.annotation_index) });
    window.__TAURI__ = { core: { invoke: async (command, raw) => {
      const args = JSON.parse(JSON.stringify(raw, (_k, v) => v instanceof Map ? Object.fromEntries(v) : v));
      window.requests.push({ command, args });
      if (command === 'document_info') {const v=view();f.infoCalls=(f.infoCalls||0)+1;if(f.infoCalls>1){if(options.syncConflict==='none')v.shape_editor=null;if(options.syncConflict==='owner')v.shape_editor={...v.shape_editor,id:9999};if(options.syncConflict==='doc')v.id=88;}return outcome(command,args,v,f.infoCalls>1?options.info:undefined);}
      if (command === 'render_page' || command === 'render_overview_page') return outcome(command, args, raster(false), f.writes&&args.scale>.25 ? options.terminalRender : undefined);
      if (command === 'notes_document') return { notes: [], unsupported_count: 0 };
      if (command === 'shapes_document') return { shapes: [], unsupported_count: 0 };
      if (command === 'free_texts_document') return { annotations: [], unsupported_count: 0 };
      if (command === 'annotations_document') return options.sidebarAnnotations||[];
      if (command === 'outline_document') return options.outline||[];
      if (command === 'register_file_drop_channel' || command === 'prepare_page_overview') return 1;
      if (command === 'canvas_shapes_document') {
        f.listCalls=(f.listCalls||0)+1;
        if (args.document_id !== f.activeId || !Number.isInteger(args.page)) throw 'invalid projection context';
        const p=projection();if(options.missingRows)delete p.hit_rows;if(options.gap){p.hit_rows=[];p.annotation_count=1;p.unsupported_count=1;}
        if(f.writes&&options.reconcileMismatch)p.annotations[0].original.line_width=31;
        return outcome(command, args, p, f.listCalls===options.holdListAt?'hold':f.writes ? options.postCommitList : options.list);
      }
      if (command === 'begin_canvas_shape_editor') {
        if(f.textOwner)throw 'overlapping Text/Shapes owner';
        if (args.document_id !== f.activeId || !Number.isInteger(args.page) || !/^[a-f0-9]{32}$/.test(args.request)) throw 'invalid begin context';
        if (args.target.kind === 'existing' && !f.selections.some(s => canonical(s) === canonical(args.target.original))) throw 'lost full selection';
        if (args.target.kind === 'new' && canonical(args.target.revision) !== canonical(f.revision)) throw 'lost revision';
        f.owner = { id: ++f.next, request: args.request, page: args.page, target: args.target };
        let reply=f.owner;if(options.wrongOwner)reply={...reply,[options.wrongOwner]:options.wrongOwner==='target'?{...args.target,revision:Array(32).fill(99)}:options.wrongOwner==='id'?0:999};
        return outcome(command, args, reply, options.begin);
      }
      if (command === 'render_shape_editor_background') {
        if (args.id !== f.owner?.id || args.request !== f.owner?.request || !(args.scale > 0)) throw 'invalid background owner';
        const png=raster(true);return outcome(command, args, options.equalPng?(f.samePng??=png):png, options.background);
      }
      if (command === 'commit_shape_editor') {
        if (canonical(Object.keys(args).sort()) !== canonical(['draft','id','request']) || !Object.hasOwn(args.draft, 'fill')) throw 'incomplete commit wire';
        if (f.committed[args.id] && canonical(f.committed[args.id]) !== canonical(args)) throw 'changed retry';
        if (options.commit==='reject') throw 'core refusal: synthetic';
        if (!f.committed[args.id]) {
          f.committed[args.id] = args; f.writes++;
          const d = args.draft, g = d.geometry, index = f.owner.target.kind === 'existing' ? f.owner.target.original.original.annotation_index : f.selections.length+(options.rows?.length||0);
          f.revision = Array(32).fill(17 + f.writes);
          const bounds = g.bounds;
          const s = { page_index: f.owner.page, page_id: [9,0], object_id: [30+index,0], revision: f.revision, flags: 4, opacity: d.opacity,
            original: { annotation_index: index, shape: g.kind, bounds, stroke: d.stroke, fill: d.fill, line_width: d.line_width, ...(g.kind === 'line' ? { line: { start: g.start, end: g.end, endings: g.endings, dash_pattern: g.dash_pattern } } : {}) } };
          f.selections = [...f.selections.filter(s => s.original.annotation_index !== index), s].map(s=>({...s,revision:f.revision}));
        }
        const result = { ...view(), shape_editor: null, ...(options.warning?{warning:'journal warning: synthetic'}:{}), ...(options.wrongTerminalId?{id:88}:{}) };
        if (!options.commit) f.owner = null;
        if(options.commit==='after-write'){f.owner=null;throw 'synthetic lost reply after write';}
        return outcome(command, args, result, options.commit);
      }
      if (command === 'cancel_canvas_shape_request' || command === 'cancel_shape_editor') { f.owner = null; return outcome(command, args, view(), options.cancel); }
      if (command === 'delete_shape_editor') {
        if (options.deleteCancel) return null;
        f.selections = f.selections.filter(s => s.original.annotation_index !== f.owner.target.original.original.annotation_index); f.writes++; f.owner = null; return view();
      }
      if(command==='begin_canvas_free_text_editor'){
        if(f.owner)throw 'overlapping Text/Shapes owner';
        f.textOwner={id:++f.next,request:args.request,page:args.page,target:args.target,appearance_style:args.target.style};return f.textOwner;
      }
      if(command==='render_free_text_editor_background')return raster(true);
      if(command==='commit_free_text_editor'){f.textOwner=null;f.textWrites=(f.textWrites||0)+1;return view();}
      if(command==='cancel_free_text_editor'||command==='cancel_canvas_free_text_request'){f.textOwner=null;return view();}
      if (command === 'activate_document') { f.activeId = args.id; return view(); }
      if (command === 'export_document') return view();
      if (command === 'prompt_page_action') return {document:null,target_page:null};
      if (command === 'edit_document') return view();
      if (command === 'close_document') return { closed: true, document: view() };
      throw new Error(`unexpected fixture command ${command}`);
    } } };
  }, options);
}
async function count(page, command) { return page.evaluate(c => window.requests.filter(r => r.command === c).length, command); }
async function open(browser, width, options = {}) {
  const page = await browser.newPage({ viewport: { width, height: 900 } }); page.setDefaultTimeout(5000);
  page.errors=[]; page.networkErrors=[]; page.consoleErrors=[];
  page.on('pageerror', e=>page.errors.push(String(e))); page.on('console', m=>{if(m.type()==='error')page.consoleErrors.push(m.text());});
  page.on('response', r=>{if(r.status()>=400)page.networkErrors.push(`${r.status()} ${r.url()}`);});
  await fixture(page, options); await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.getByRole('button',{name:'文档标签 synthetic.pdf',exact:true}).waitFor(); return page;
}
async function activate(page) {
  await page.getByRole('button',{name:'批注',exact:true}).click();
  await page.getByRole('button',{name:'工具 · 形状',exact:true}).click();
  assert.equal(await page.locator('.inspector:visible').count(),0,'Shapes must not open old inspector');
  await page.getByRole('group',{name:'形状样式',exact:true}).waitFor();
  await page.waitForFunction(()=>window.requests.some(r=>r.command==='canvas_shapes_document')&&document.querySelector('#pdf-page-image')?.complete);
}
async function waitCount(page,command,n=1){await page.waitForFunction(({command,n})=>window.requests.filter(r=>r.command===command).length>=n,{command,n});}
async function release(page,command,override){await page.evaluate(({command,override})=>{const p=window.fixture.pending.find(p=>p.command===command&&!p.settled);if(!p)throw `no pending ${command}`;p.settled=true;if(override?.reject)p.reject(override.reject);else p.resolve(override?.value??p.value);},{command,override});}
async function box(page){await page.waitForFunction(()=>document.querySelector('#pdf-page-image')?.complete&&document.querySelector('#pdf-page-image')?.naturalWidth>0);return page.locator('#pdf-page-image').boundingBox();}
async function drag(page,a=[.3,.3],z=[.6,.5],up=true){const b=await box(page);await page.mouse.move(b.x+b.width*a[0],b.y+b.height*a[1]);await page.mouse.down();await page.mouse.move(b.x+b.width*z[0],b.y+b.height*z[1],{steps:5});if(up)await page.mouse.up();return b;}
async function settled(page,writes=1){await page.waitForFunction(w=>window.fixture.writes===w&&!window.fixture.owner,writes);await page.waitForFunction(()=>!document.querySelector('.canvas-shapes-recovery'));}
const red={r:1,g:Math.fround(66/255),b:Math.fround(69/255)};
const existing=(kind='rectangle',extra={})=>({page_index:0,page_id:[9,0],object_id:[30,0],revision:Array(32).fill(17),flags:4,opacity:1,original:{annotation_index:0,shape:kind,bounds:{x:180,y:300,width:180,height:140},stroke:red,fill:null,line_width:2,...(kind==='line'?{line:{start:{x:200,y:320},end:{x:340,y:420},endings:['none','open_arrow'],dash_pattern:[3,2]}}:{})},...extra});
const row=(subtype,bounds,index=0)=>({page_index:0,annotation_index:index,page_id:[9,0],object_id:null,subtype,contents:'',color:null,bounds,modified:null});
const cases = [['direct', {}, async page=>{
  await activate(page);
  await page.waitForFunction(()=>window.requests.some(r=>r.command==='canvas_shapes_document')&&document.querySelector('#pdf-page-image')?.complete);
  const b=await page.locator('#pdf-page-image').boundingBox();
  await page.mouse.click(b.x+b.width*.4,b.y+b.height*.4); assert.equal(await count(page,'begin_canvas_shape_editor'),0,'blank click creates nothing');
  await page.mouse.move(b.x+b.width*.3,b.y+b.height*.3); await page.mouse.down(); await page.mouse.move(b.x+b.width*.6,b.y+b.height*.5,{steps:5}); await page.mouse.up();
  await page.waitForFunction(()=>window.fixture.writes===1&&!window.fixture.owner);
  await page.waitForFunction(()=>document.querySelectorAll('.canvas-shape-handle').length===4);
  assert.equal(await count(page,'commit_shape_editor'),1); assert.equal(await count(page,'begin_canvas_shape_editor'),1);
}],
['blocking-I1-old-refusal-proof-after-exact-retry',{begin:'hold',commit:'reject',info:'hold'},async page=>{
  await activate(page);await drag(page);await page.getByRole('button',{name:'下一页',exact:true}).click();
  await release(page,'begin_canvas_shape_editor');
  await page.getByRole('button',{name:'精确重试形状',exact:true}).waitFor();await waitCount(page,'document_info',2);
  const first=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args);
  await page.evaluate(()=>window.fixture.options.commit='hold');await page.getByRole('button',{name:'精确重试形状',exact:true}).click();await waitCount(page,'commit_shape_editor',2);
  await release(page,'document_info');
  assert.equal(await page.getByRole('spinbutton',{name:'形状线宽',exact:true}).isDisabled(),true,'old refusal proof must not downgrade newer committing retry');
  assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'),'PDF 第 1 页');
  await page.evaluate(()=>window.fixture.owner=null);await release(page,'commit_shape_editor');await settled(page);
  await page.waitForFunction(()=>document.querySelector('#pdf-page-image')?.alt==='PDF 第 2 页');
  assert.deepEqual(await page.evaluate(()=>window.requests.filter(r=>r.command==='commit_shape_editor').map(r=>r.args)),[first,first]);
  assert.equal(await page.evaluate(()=>window.fixture.writes),1);assert.equal(await count(page,'begin_canvas_shape_editor'),1);
  assert.equal(await page.evaluate(()=>window.requests.filter(r=>r.command==='render_page'&&r.args.page===1&&r.args.scale>.25).length),1,'first navigation intent executes exactly once (exclude thumbnails)');
}],
['blocking-I2-cold-new-nonempty-append',{list:'reject',selections:[existing()],rows:[row('Popup',{x:10,y:10,width:20,height:20},1)],owner:{id:41,request:'d'.repeat(32),page:0,target:{kind:'new',draft:{geometry:{kind:'rectangle',bounds:{x:380,y:350,width:100,height:100}},stroke:red,fill:null,line_width:2,opacity:1},revision:Array(32).fill(17)}}},async page=>{
  await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('core refusal'));await page.getByRole('spinbutton',{name:'形状线宽',exact:true}).fill('3');await page.getByRole('spinbutton',{name:'形状线宽',exact:true}).dispatchEvent('change');assert.equal(await count(page,'commit_shape_editor'),0,'missing recovered append projection blocks submission');
  await page.evaluate(()=>{window.fixture.options.list=undefined;window.fixture.options.projectionRevision=Array(32).fill(99);});await page.getByRole('button',{name:'重试形状背景',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('版本不匹配'));assert.equal(await count(page,'commit_shape_editor'),0,'wrong recovered revision blocks submission');
  await page.evaluate(()=>window.fixture.options.projectionRevision=undefined);await page.getByRole('button',{name:'重试形状背景',exact:true}).click();
  await waitCount(page,'commit_shape_editor');await settled(page);assert.equal(await count(page,'begin_canvas_shape_editor'),0);assert.equal(await count(page,'commit_shape_editor'),1);
  const saved=await page.evaluate(()=>window.fixture.selections);assert.deepEqual(saved.map(s=>s.original.annotation_index),[0,2]);assert.equal(saved[1].original.line_width,3);
  assert.equal(await page.locator('.canvas-shape-handle').count(),4);const b=await box(page);const h=await page.locator('.canvas-shape-handle').first().boundingBox();assert.ok(h.x>b.x+b.width*.6,'fresh selection must be appended occurrence, not preexisting index zero');
}],
['blocking-I3-imported-selection-creation-defaults',{selections:[existing('line',{opacity:.5,original:{...existing('line').original,line_width:32,fill:{r:1,g:1,b:0}}})]},async page=>{
  await activate(page);
  for(const exit of ['blank','kind']){
    const b=await box(page);await page.mouse.click(b.x+b.width*270/595,b.y+b.height*(842-370)/842);
    const width=page.getByRole('spinbutton',{name:'形状线宽',exact:true}),alpha=page.getByRole('spinbutton',{name:'形状透明度',exact:true});assert.equal(await width.inputValue(),'32');assert.equal(await alpha.inputValue(),'50');
    if(exit==='blank')await page.mouse.click(b.x+b.width*.8,b.y+b.height*.2);else await page.getByRole('button',{name:'椭圆',exact:true}).click();
    const displayed={width:Number(await width.inputValue()),opacity:1-Number(await alpha.inputValue())/100};
    await drag(page,exit==='blank'?[.7,.2]:[.7,.65],exit==='blank'?[.9,.35]:[.9,.8]);await settled(page,exit==='blank'?1:2);
    const d=await page.evaluate(()=>window.requests.filter(r=>r.command==='commit_shape_editor').at(-1).args.draft);
    assert.equal(d.line_width,displayed.width,'visible creation width must match saved payload after '+exit);assert.equal(d.opacity,displayed.opacity,'visible creation opacity must match saved payload after '+exit);
    assert.equal(d.line_width,2);assert.equal(d.opacity,1);assert.equal(d.fill,null);assert.deepEqual(d.stroke,red);assert.equal(d.geometry.kind,exit==='blank'?'rectangle':'ellipse');
  }
  const imported=await page.evaluate(()=>window.fixture.selections.find(s=>s.original.annotation_index===0));assert.equal(imported.original.line_width,32);assert.equal(imported.opacity,.5);assert.deepEqual(imported.original.line.endings,['none','open_arrow']);assert.deepEqual(imported.original.line.dash_pattern,[3,2]);assert.equal(await count(page,'commit_shape_editor'),2);
}],
['blocking-I4-sync-owner-does-not-leak',{selections:[existing()],info:'hold'},async page=>{
  await activate(page);const b=await box(page);await page.mouse.click(b.x+b.width*270/595,b.y+b.height*(842-370)/842);
  const slider=page.getByRole('slider',{name:'形状线宽滑块',exact:true}),sync=page.getByRole('button',{name:'同步形状状态',exact:true});
  await slider.evaluate(e=>{e.value='3';e.dispatchEvent(new Event('input',{bubbles:true}));});await page.waitForFunction(()=>document.querySelector('.canvas-shape-preview path'));await sync.click();await waitCount(page,'document_info',2);
  await slider.dispatchEvent('change');await settled(page);
  await slider.evaluate(e=>{e.value='4';e.dispatchEvent(new Event('input',{bubbles:true}));});await page.waitForFunction(()=>document.querySelector('.canvas-shape-preview path'));
  assert.equal(await sync.isDisabled(),false,'completed owner must release its sync marker');await sync.click();await waitCount(page,'document_info',3);
  await release(page,'document_info');assert.equal(await sync.isDisabled(),true,'late predecessor callback cannot clear successor sync marker');
  await release(page,'document_info');await page.waitForFunction(()=>!document.querySelector('.canvas-shapes-recovery button:nth-last-child(2)').disabled);assert.equal(await sync.isDisabled(),false);
  await slider.dispatchEvent('change');await settled(page,2);assert.equal(await count(page,'commit_shape_editor'),2);assert.equal(await page.evaluate(()=>window.fixture.writes),2);
}],
['delayed-begin-final-pointerup-first-intent',{begin:'hold'},async page=>{
  await activate(page);const b=await drag(page);await waitCount(page,'begin_canvas_shape_editor');assert.equal(await count(page,'commit_shape_editor'),0);
  await page.getByRole('button',{name:'下一页',exact:true}).click();await page.getByRole('button',{name:'工具 · 便笺',exact:true}).click();
  assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'),'PDF 第 1 页');
  await release(page,'begin_canvas_shape_editor');await settled(page);await page.waitForFunction(()=>document.querySelector('#pdf-page-image')?.alt==='PDF 第 2 页');
  const d=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args.draft);assert.ok(Math.abs(d.geometry.bounds.width-178.5)<2);assert.equal(await count(page,'commit_shape_editor'),1);
}],
['stale-stage-timer-after-ready',{stageTimers:true},async page=>{
  await activate(page);await drag(page,[.3,.3],[.5,.5],false);await page.waitForFunction(()=>document.querySelector('.canvas-shape-preview path'));
  await page.evaluate(()=>{for(const timer of window.fixture.timers.values())timer();});
  assert.ok(!(await page.getByRole('group',{name:'形状事务恢复'}).innerText()).includes('解码超时'),'decoded ready stage timers must be inert');
  await page.mouse.up();await settled(page);
}],
['background-rejected-retry',{background:'reject'},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('core refusal'));assert.equal(await count(page,'commit_shape_editor'),0);
  await page.evaluate(()=>window.fixture.options.background=undefined);await page.getByRole('button',{name:'重试形状背景',exact:true}).click();await settled(page);assert.equal(await count(page,'begin_canvas_shape_editor'),1);
}],
['frozen-retry-timeout',{commit:'timeout'},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('未知'));
  const first=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args);assert.equal(await page.getByRole('spinbutton',{name:'形状线宽',exact:true}).isDisabled(),true);
  await page.evaluate(()=>window.fixture.options.commit=undefined);await page.getByRole('button',{name:'精确重试形状',exact:true}).click();await settled(page);assert.deepEqual(await page.evaluate(()=>window.requests.filter(r=>r.command==='commit_shape_editor').map(r=>r.args)),[first,first]);
}],
['cancel-preack-late-ack',{begin:'hold'},async page=>{
  await activate(page);await drag(page);await waitCount(page,'begin_canvas_shape_editor');await page.keyboard.press('Escape');await waitCount(page,'cancel_canvas_shape_request');await settled(page,0);
  await release(page,'begin_canvas_shape_editor');assert.equal(await count(page,'render_shape_editor_background'),0);assert.equal(await count(page,'commit_shape_editor'),0);
}],
['cancel-owner-escape',{},async page=>{
  await activate(page);await drag(page,[.3,.3],[.5,.5],false);await page.waitForFunction(()=>document.querySelector('.canvas-shape-preview path'));await page.keyboard.press('Escape');await page.mouse.up();await settled(page,0);assert.equal(await count(page,'cancel_shape_editor'),1);assert.equal(await count(page,'commit_shape_editor'),0);
}],
['terminal-projection-retry',{postCommitList:'reject'},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('仅重试完成投影'));assert.equal(await count(page,'commit_shape_editor'),1);assert.equal(await page.getByRole('button',{name:'工具 · 便笺',exact:true}).isDisabled(),true);
  await page.evaluate(()=>window.fixture.options.postCommitList=undefined);await page.getByRole('button',{name:'重试形状投影',exact:true}).click();await settled(page);assert.equal(await count(page,'commit_shape_editor'),1);assert.equal(await page.locator('.canvas-shape-handle').count(),4);
}],
['selected-move-resize',{selections:[existing()]},async page=>{
  await activate(page);let b=await box(page);await page.mouse.click(b.x+b.width*270/595,b.y+b.height*(842-370)/842);assert.equal(await count(page,'begin_canvas_shape_editor'),0);assert.equal(await page.locator('.canvas-shape-handle').count(),4);
  await drag(page,[270/595,(842-370)/842],[290/595,(842-390)/842]);await settled(page);assert.equal(await count(page,'commit_shape_editor'),1);
  let handles=await page.locator('.canvas-shape-handle').all();const h=await handles[0].boundingBox();await page.mouse.move(h.x+h.width/2,h.y+h.height/2);await page.mouse.down();await page.mouse.move(h.x-12,h.y+12,{steps:4});await page.mouse.up();await settled(page,2);assert.equal(await count(page,'commit_shape_editor'),2);
}],
['styles-continuous-fill-memory',{},async page=>{
  await activate(page);await page.getByRole('checkbox',{name:'启用形状填充',exact:true}).check();let width=page.getByRole('spinbutton',{name:'形状线宽',exact:true});await width.fill('32');await width.dispatchEvent('change');assert.equal(await width.inputValue(),'32');
  await page.getByRole('button',{name:'直线',exact:true}).click();assert.equal(await page.getByRole('checkbox',{name:'启用形状填充',exact:true}).isDisabled(),true);
  await page.getByRole('button',{name:'矩形',exact:true}).click();assert.equal(await page.getByRole('checkbox',{name:'启用形状填充',exact:true}).isChecked(),true);await drag(page);await settled(page);
  const d=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args.draft);assert.equal(d.line_width,32);assert.notEqual(d.fill,null);assert.equal(d.stroke.r,1);assert.equal(await page.getByRole('group',{name:'形状样式',exact:true}).evaluate(e=>e.getBoundingClientRect().height),42);
}],
['dark-controls-and-visible-fill-preview',{commit:'hold'},async page=>{
  await activate(page);const background=await page.getByRole('spinbutton',{name:'形状线宽',exact:true}).evaluate(e=>getComputedStyle(e).backgroundColor);assert.notEqual(background,'rgb(255, 255, 255)','numeric control must inherit dark chrome');
  await page.getByRole('checkbox',{name:'启用形状填充',exact:true}).check();await drag(page);await waitCount(page,'commit_shape_editor');
  const p=page.locator('.canvas-shape-preview path');assert.equal(await p.count(),1);assert.equal(await p.getAttribute('stroke'),'#ff4245');assert.equal(await p.getAttribute('fill'),'#ffd600');assert.equal(await p.getAttribute('fill-opacity'),'1');assert.equal(await p.getAttribute('stroke-opacity'),'1');
  await page.screenshot({path:path.join(evidence,`held-visible-fill-preview-${page.viewportSize().width}.png`),fullPage:true});
  await page.evaluate(()=>window.fixture.owner=null);await release(page,'commit_shape_editor');await settled(page);
}],
['invalid-numeric-refuses-stale-candidate',{selections:[existing()]},async page=>{
  await activate(page);const b=await box(page);await page.mouse.click(b.x+b.width*270/595,b.y+b.height*(842-370)/842);
  const field=page.getByRole('spinbutton',{name:'形状线宽',exact:true});await page.getByRole('slider',{name:'形状线宽滑块',exact:true}).evaluate(e=>{e.value='3';e.dispatchEvent(new Event('input',{bubbles:true}));});await page.waitForFunction(()=>document.querySelector('.canvas-shape-preview path'));
  await field.fill('');await field.dispatchEvent('change');await page.getByRole('button',{name:'下一页',exact:true}).click();
  assert.equal(await count(page,'commit_shape_editor'),0,'invalid numeric field must not commit stale candidate');assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'),'PDF 第 1 页','invalid numeric field must not navigate using stale committed style');assert.equal(await field.inputValue(),'');
}],
['outside-final-refuses-stale-draft',{},async page=>{
  await activate(page);await drag(page,[.3,.3],[.5,.5],false);await page.waitForFunction(()=>document.querySelector('.canvas-shape-preview path'));const b=await box(page);await page.mouse.move(b.x-15,b.y+20);await page.mouse.up();
  assert.equal(await count(page,'commit_shape_editor'),0,'invalid final point cannot save last valid intermediate draft');
}],
['rejected-action-clears-first-intent',{commit:'reject'},async page=>{
  await activate(page);await drag(page);await page.getByRole('button',{name:'下一页',exact:true}).click();await page.waitForFunction(()=>window.requests.filter(r=>r.command==='document_info').length>=2);
  await page.evaluate(()=>window.fixture.options.commit=undefined);await page.getByRole('button',{name:'工具 · 便笺',exact:true}).click();await settled(page);await page.waitForFunction(()=>document.querySelector('[aria-label="工具 · 便笺"]')?.getAttribute('aria-pressed')==='true');assert.equal(await page.locator('#pdf-page-image').getAttribute('alt'),'PDF 第 1 页');
}],
...['malformed','after-write'].map(commit=>[`uncertain-${commit}-exact-retry`,{commit},async page=>{await activate(page);await drag(page);await page.getByRole('button',{name:'精确重试形状',exact:true}).waitFor();const first=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args);await page.evaluate(()=>window.fixture.options.commit=undefined);await page.getByRole('button',{name:'精确重试形状',exact:true}).click();await settled(page);assert.deepEqual(await page.evaluate(()=>window.requests.filter(r=>r.command==='commit_shape_editor').map(r=>r.args)),[first,first]);}]),
['matching-sync-retains-local-candidate',{selections:[existing()]},async page=>{
  await activate(page);const b=await box(page);await page.mouse.click(b.x+b.width*270/595,b.y+b.height*(842-370)/842);await page.getByRole('slider',{name:'形状线宽滑块',exact:true}).evaluate(e=>{e.value='3';e.dispatchEvent(new Event('input',{bubbles:true}));});await page.waitForFunction(()=>document.querySelector('.canvas-shape-preview path'));await page.getByRole('button',{name:'同步形状状态',exact:true}).click();await waitCount(page,'document_info',2);assert.equal(await count(page,'commit_shape_editor'),0);await page.getByRole('button',{name:'工具 · 便笺',exact:true}).click();await settled(page);const d=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args.draft);assert.equal(d.line_width,3);
}],
['handoff-render-held-blocks-actions',{terminalRender:'hold'},async page=>{
  await activate(page);await drag(page);await waitCount(page,'commit_shape_editor');await page.waitForFunction(()=>window.fixture.pending.some(p=>p.command==='render_page'));assert.equal(await page.getByRole('button',{name:'下一页',exact:true}).isDisabled(),true);await drag(page);assert.equal(await count(page,'begin_canvas_shape_editor'),1);await release(page,'render_page');await settled(page);
}],
['handoff-projection-held-blocks-actions',{postCommitList:'hold'},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>window.fixture.pending.some(p=>p.command==='canvas_shapes_document'));assert.equal(await page.getByRole('button',{name:'工具 · 便笺',exact:true}).isDisabled(),true);await release(page,'canvas_shapes_document');await settled(page);
}],
['current-primary-image-error-retains-readonly-handoff',{postCommitList:'hold'},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>window.fixture.pending.some(p=>p.command==='canvas_shapes_document'));
  // Simulate the current primary decoder's failure, not an old detached image.
  await page.locator('#pdf-page-image').evaluate(e=>{Object.defineProperty(e,'naturalWidth',{configurable:true,value:0});e.dispatchEvent(new Event('error'));});
  assert.ok((await page.getByRole('group',{name:'形状事务恢复',exact:true}).innerText()).includes('解码失败'),'current primary decode error must reach Shapes recovery immediately');
  await release(page,'canvas_shapes_document');assert.equal(await page.locator('.canvas-shapes-recovery').count(),1,'old projection cannot finish a failed current decoder');assert.equal(await count(page,'commit_shape_editor'),1);
  await page.locator('#pdf-page-image').evaluate(e=>{delete e.naturalWidth;});await page.evaluate(()=>window.fixture.options.postCommitList=undefined);await page.getByRole('button',{name:'重试形状背景',exact:true}).click();await settled(page);assert.equal(await count(page,'commit_shape_editor'),1);
}],
['equal-source-old-primary-error-is-inert',{postCommitList:'hold'},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>window.fixture.pending.some(p=>p.command==='canvas_shapes_document'));await page.locator('#pdf-page-image').evaluate(e=>window.oldPrimary=e);
  await page.getByRole('button',{name:'重试形状背景',exact:true}).click();await waitCount(page,'canvas_shapes_document',4);await page.evaluate(()=>window.oldPrimary.dispatchEvent(new Event('error')));
  assert.ok(!(await page.getByRole('group',{name:'形状事务恢复',exact:true}).innerText()).includes('解码失败'),'old equal-source primary callback cannot fail successor attempt');
  await release(page,'canvas_shapes_document');assert.equal(await page.locator('.canvas-shapes-recovery').count(),1);await release(page,'canvas_shapes_document');await settled(page);assert.equal(await count(page,'commit_shape_editor'),1);
}],
['handoff-reconcile-mismatch-readonly-retry',{reconcileMismatch:true},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('does not match'));assert.equal(await count(page,'commit_shape_editor'),1);await page.evaluate(()=>{window.fixture.options.reconcileMismatch=false;window.fixture.selections[0].original.line_width=2;});await page.getByRole('button',{name:'重试形状投影',exact:true}).click();await settled(page);assert.equal(await count(page,'commit_shape_editor'),1);
}],
['equal-png-background-attempts',{background:'hold',equalPng:true,stageTimers:true},async page=>{
  await activate(page);await drag(page);await waitCount(page,'render_shape_editor_background');await page.getByRole('button',{name:'重试形状背景',exact:true}).click();await waitCount(page,'render_shape_editor_background',2);
  await release(page,'render_shape_editor_background',{reject:'old failed attempt'});await release(page,'render_shape_editor_background');await settled(page);assert.equal(await count(page,'begin_canvas_shape_editor'),1);assert.equal(await count(page,'commit_shape_editor'),1);
}],
['warning-success-stays-success',{warning:true},async page=>{await activate(page);await drag(page);await settled(page);assert.ok((await page.locator('.statusbar').innerText()).includes('journal warning'));assert.equal(await count(page,'commit_shape_editor'),1);} ],
...['pointercancel','lostpointercapture'].map(type=>[`cancel-${type}`,{},async page=>{await activate(page);await drag(page,[.3,.3],[.5,.5],false);await page.waitForFunction(()=>document.querySelector('.canvas-shape-preview path'));await page.locator('.drawing-surface').evaluate((e,type)=>e.dispatchEvent(new PointerEvent(type,{bubbles:true,pointerId:1})),type);await page.mouse.up();await settled(page,0);assert.equal(await count(page,'cancel_shape_editor'),1);assert.equal(await count(page,'commit_shape_editor'),0);}]),
...['0','50','100'].map(alpha=>[`alpha-${alpha}`,{},async page=>{await activate(page);const f=page.getByRole('spinbutton',{name:'形状透明度',exact:true});await f.fill(alpha);await f.dispatchEvent('change');await drag(page);await settled(page);const d=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args.draft);assert.equal(d.opacity,1-Number(alpha)/100);assert.equal(await page.locator('.canvas-shape-handle').count(),4);}]),
['text-ime-to-shapes-final-input',{},async page=>{
  await page.getByRole('button',{name:'批注',exact:true}).click();await page.getByRole('button',{name:'工具 · 文字批注',exact:true}).click();let b=await box(page);await page.mouse.click(b.x+b.width*.5,b.y+b.height*.4);const text=page.getByRole('textbox',{name:'页内文字正文',exact:true});await text.waitFor();await text.fill('组合输入');await text.dispatchEvent('compositionstart');await page.getByRole('button',{name:'工具 · 形状',exact:true}).click();assert.equal(await count(page,'commit_free_text_editor'),0);await text.fill('组合输入最终汉字 🦀');await text.dispatchEvent('compositionend');
  await page.waitForFunction(()=>document.querySelector('[aria-label="工具 · 形状"]')?.getAttribute('aria-pressed')==='true');assert.equal(await page.evaluate(()=>window.requests.find(r=>r.command==='commit_free_text_editor').args.contents),'组合输入最终汉字 🦀');
  await drag(page);await settled(page);assert.equal(await count(page,'commit_shape_editor'),1);assert.equal(await count(page,'commit_free_text_editor'),1);
}],
['shapes-to-text-handoff',{commit:'hold'},async page=>{
  await activate(page);await drag(page);await waitCount(page,'commit_shape_editor');await page.getByRole('button',{name:'工具 · 文字批注',exact:true}).click();assert.equal(await count(page,'begin_canvas_free_text_editor'),0);await page.evaluate(()=>window.fixture.owner=null);await release(page,'commit_shape_editor');await page.waitForFunction(()=>document.querySelector('[aria-label="工具 · 文字批注"]')?.getAttribute('aria-pressed')==='true');const b=await box(page);await page.mouse.click(b.x+b.width*.5,b.y+b.height*.4);await page.getByRole('textbox',{name:'页内文字正文',exact:true}).waitFor();assert.equal(await count(page,'begin_canvas_free_text_editor'),1);
}],
['cold-native-owner-restores-page-mode',{owner:{id:41,request:'d'.repeat(32),page:1,target:{kind:'new',draft:{geometry:{kind:'rectangle',bounds:{x:150,y:350,width:100,height:100}},stroke:red,fill:null,line_width:2,opacity:1},revision:Array(32).fill(17)}}},async page=>{
  await page.waitForFunction(()=>document.querySelector('#pdf-page-image')?.alt==='PDF 第 2 页');assert.equal(await page.getByRole('button',{name:'工具 · 形状',exact:true}).getAttribute('aria-pressed'),'true');await waitCount(page,'render_shape_editor_background');assert.equal(await count(page,'begin_canvas_shape_editor'),0);assert.ok((await page.locator('.statusbar').innerText()).includes('未提交'),'cold owner must visibly warn lost local input');
}],
['simultaneous-text-shape-owners-invalid',{owner:{id:41,request:'d'.repeat(32),page:0,target:{kind:'new',draft:{geometry:{kind:'rectangle',bounds:{x:150,y:350,width:100,height:100}},stroke:red,fill:null,line_width:2,opacity:1},revision:Array(32).fill(17)}},textOwner:{id:42,request:'e'.repeat(32),page:0,target:{kind:'new',bounds:{x:200,y:350,width:150,height:100},style:{font:'sans',font_size:18,color:{r:0,g:0,b:0},alignment:'left',line_height:1.3}},appearance_style:{font:'sans',font_size:18,color:{r:0,g:0,b:0},alignment:'left',line_height:1.3}}},async page=>{
  await page.getByRole('group',{name:'形状事务恢复',exact:true}).waitFor();assert.ok((await page.getByRole('group',{name:'形状事务恢复',exact:true}).innerText()).includes('冲突'));assert.equal(await count(page,'render_shape_editor_background'),0);assert.equal(await count(page,'render_free_text_editor_background'),0);
}],
...['tool','mode','page','tab','route','zoom','fit','fit-width','sidebar','overview','panel','page-prompt','close','export'].map(intent=>[`intent-once-${intent}`,{commit:'hold'},async page=>{
  await activate(page);await drag(page);await waitCount(page,'commit_shape_editor');const before=await page.getByRole('button',{name:'缩放',exact:true}).count();
  const action=async()=>{
    if(intent==='tool')await page.getByRole('button',{name:'工具 · 便笺',exact:true}).click();
    if(intent==='mode')await page.getByRole('button',{name:'编辑',exact:true}).click();
    if(intent==='page')await page.getByRole('button',{name:'下一页',exact:true}).click();
    if(intent==='tab')await page.getByRole('button',{name:'文档标签 second.pdf',exact:true}).click();
    if(intent==='route')await page.getByRole('button',{name:'主界面标签',exact:true}).click();
    if(intent==='zoom')await page.getByRole('button',{name:'放大',exact:true}).click();
    if(intent==='fit')await page.getByRole('button',{name:'适合页面',exact:true}).click();
    if(intent==='fit-width')await page.getByRole('combobox',{name:'缩放',exact:true}).selectOption('width');
    if(intent==='sidebar')await page.getByRole('button',{name:'侧栏',exact:true}).click();
    if(intent==='overview')await page.getByRole('button',{name:'页面预览',exact:true}).click();
    if(intent==='close')await page.getByRole('button',{name:'关闭 synthetic.pdf',exact:true}).click();
    if(intent==='export')await page.getByRole('button',{name:'导出',exact:true}).click();
    if(intent==='panel'||intent==='page-prompt'){
      const summary=page.getByLabel('更多工具',{exact:true});if(!await summary.evaluate(e=>e.parentElement.open))await summary.click();
      await page.getByRole('button',{name:intent==='panel'?'文档操作':'页面操作',exact:true}).click();
      await page.getByRole('button',{name:intent==='panel'?'导出副本':'移动当前页…',exact:true}).click();
    }
  };
  const initialZoom=await page.getByRole('combobox',{name:'缩放',exact:true}).innerText();const sidebar=await page.getByRole('button',{name:'侧栏',exact:true}).getAttribute('aria-pressed');
  await action();await action();assert.equal(await count(page,'commit_shape_editor'),1);assert.equal(await page.getByRole('button',{name:'工具 · 形状',exact:true}).getAttribute('aria-pressed'),'true','captured action must wait for handoff');
  if(intent==='page-prompt')assert.equal(await count(page,'prompt_page_action'),0,'native input waits for Shapes handoff');
  if(intent==='panel')assert.equal(await count(page,'export_document'),0,'File export waits for Shapes handoff');
  await page.evaluate(()=>window.fixture.owner=null);await release(page,'commit_shape_editor');await settled(page);
  if(intent==='tool')await page.waitForFunction(()=>document.querySelector('[aria-label="工具 · 便笺"]')?.getAttribute('aria-pressed')==='true');
  if(intent==='mode')assert.equal(await page.getByRole('button',{name:'编辑',exact:true}).getAttribute('aria-pressed'),'true');
  if(intent==='page')await page.waitForFunction(()=>document.querySelector('#pdf-page-image')?.alt==='PDF 第 2 页');
  if(intent==='tab')assert.equal(await count(page,'activate_document'),1);
  if(intent==='route')assert.equal(await page.locator('#reader-workspace').isHidden(),true);
  if(intent==='sidebar')assert.notEqual(await page.getByRole('button',{name:'侧栏',exact:true}).getAttribute('aria-pressed'),sidebar,'sidebar toggles exactly once');
  if(intent==='overview')assert.equal(await page.getByRole('button',{name:'页面预览',exact:true}).getAttribute('aria-pressed'),'true');
  if(intent==='panel'){await waitCount(page,'export_document');assert.equal(await count(page,'export_document'),1);assert.equal(await page.locator('.inspector').count(),0);}
  if(intent==='page-prompt'){await waitCount(page,'prompt_page_action');assert.equal(await count(page,'prompt_page_action'),1);assert.deepEqual(await page.evaluate(()=>window.requests.find(r=>r.command==='prompt_page_action').args),{operation:'move',page:0});}
  if(intent==='close')assert.equal(await count(page,'close_document'),1);
  if(intent==='export'){await page.getByRole('button',{name:'工具 · 存储副本',exact:true}).click();await waitCount(page,'export_document');assert.equal(await count(page,'export_document'),1);}
  if(intent==='zoom'){const text=await page.getByRole('combobox',{name:'缩放',exact:true}).innerText();assert.notEqual(text,initialZoom);}
  assert.equal(await count(page,'commit_shape_editor'),1);
}]),
...['outline','annotation'].map(kind=>[`intent-${kind}-once`,{commit:'hold',outline:[{title:'destination two',depth:0,path:[0],page_index:1}],sidebarAnnotations:[row('Text',{x:20,y:20,width:50,height:50})].map(r=>({...r,page_index:1}))},async page=>{
  if(page.viewportSize().width<=720)await page.getByRole('button',{name:'侧栏',exact:true}).click();await page.getByRole('button',{name:kind==='outline'?'大纲':'侧栏批注',exact:true}).click();
  await activate(page);await drag(page,[.65,.3],[.9,.5]);await waitCount(page,'commit_shape_editor');const target=page.locator(kind==='outline'?'.outline-row':'.annotation-row').first();await target.click();await target.click();await page.evaluate(()=>window.fixture.owner=null);await release(page,'commit_shape_editor');await settled(page);await page.waitForFunction(()=>document.querySelector('#pdf-page-image')?.alt==='PDF 第 2 页');assert.equal(await count(page,'commit_shape_editor'),1);
}]),
...['timeout','malformed'].map(begin=>[`begin-${begin}-cancel-request`,{begin},async page=>{await activate(page);await drag(page);await page.waitForFunction(text=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes(text),begin==='timeout'?'未知':'格式');assert.equal(await count(page,'commit_shape_editor'),0);await page.getByRole('button',{name:'取消／同步形状',exact:true}).click();await settled(page,0);assert.equal(await count(page,'cancel_canvas_shape_request'),1);}]),
['context-image-change-before-ack-refuses',{begin:'hold'},async page=>{
  await activate(page);await drag(page);await waitCount(page,'begin_canvas_shape_editor');await page.locator('#pdf-page-image').evaluate(e=>{const canvas=document.createElement('canvas');canvas.width=595;canvas.height=842;e.src=canvas.toDataURL();});await release(page,'begin_canvas_shape_editor');await page.waitForFunction(()=>window.requests.some(r=>r.command==='cancel_canvas_shape_request'||r.command==='cancel_shape_editor'));assert.equal(await count(page,'commit_shape_editor'),0);
}],
['cancel-rejected-can-exact-retry',{begin:'hold',cancel:'reject'},async page=>{
  await activate(page);await drag(page);await waitCount(page,'begin_canvas_shape_editor');await page.keyboard.press('Escape');await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('core refusal'));await page.evaluate(()=>window.fixture.options.cancel=undefined);await page.getByRole('button',{name:'取消／同步形状',exact:true}).click();await waitCount(page,'cancel_canvas_shape_request',2);await settled(page,0);const args=await page.evaluate(()=>window.requests.filter(r=>r.command==='cancel_canvas_shape_request').map(r=>r.args));assert.deepEqual(args[0],args[1]);
}],
['wrong-document-terminal-is-uncertain',{wrongTerminalId:true},async page=>{await activate(page);await drag(page);await page.getByRole('button',{name:'精确重试形状',exact:true}).waitFor();await page.evaluate(()=>window.fixture.options.wrongTerminalId=false);await page.getByRole('button',{name:'精确重试形状',exact:true}).click();await settled(page);assert.equal(await count(page,'commit_shape_editor'),2);assert.equal(await page.evaluate(()=>window.fixture.writes),1);} ],
['invalid-decode-old-attempt-inert',{background:'invalid-image',stageTimers:true},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('解码失败'));await page.evaluate(()=>{window.oldStage=document.querySelector('.canvas-shape-staged');window.oldTimers=[...window.fixture.timers.values()];window.fixture.options.background='hold';});await page.getByRole('button',{name:'重试形状背景',exact:true}).click();await waitCount(page,'render_shape_editor_background',2);
  await page.evaluate(()=>{window.oldStage.dispatchEvent(new Event('load'));window.oldStage.dispatchEvent(new Event('error'));for(const t of window.oldTimers)t();});assert.ok(!(await page.getByRole('group',{name:'形状事务恢复',exact:true}).innerText()).includes('解码失败'));assert.equal(await count(page,'commit_shape_editor'),0);await release(page,'render_shape_editor_background');await settled(page);assert.equal(await count(page,'begin_canvas_shape_editor'),1);
}],
['projection-stale-error-cannot-poison-reactivation',{holdListAt:2},async page=>{
  await activate(page);await waitCount(page,'canvas_shapes_document',2);await page.getByRole('button',{name:'工具 · 形状',exact:true}).click();await page.getByRole('button',{name:'工具 · 形状',exact:true}).click();await waitCount(page,'canvas_shapes_document',3);await release(page,'canvas_shapes_document',{reject:'old projection failure'});await drag(page);await settled(page);assert.equal(await count(page,'commit_shape_editor'),1);assert.ok(!(await page.locator('.statusbar').innerText()).includes('old projection failure'));
}],
['projection-stale-completion-cannot-poison-successor',{holdListAt:3},async page=>{
  await activate(page);await drag(page);await page.waitForFunction(()=>window.fixture.pending.some(p=>p.command==='canvas_shapes_document'));await page.getByRole('button',{name:'重试形状背景',exact:true}).click();await settled(page);await release(page,'canvas_shapes_document',{reject:'old completion failure'});assert.equal(await page.locator('.canvas-shapes-recovery').count(),0);await drag(page,[.7,.3],[.9,.5]);await settled(page,2);assert.equal(await count(page,'commit_shape_editor'),2);
}],
['live-sample-follows-kind',{},async page=>{await activate(page);await page.getByRole('button',{name:'椭圆',exact:true}).click();assert.ok((await page.locator('.shape-live-sample path').first().getAttribute('d')).includes(' C '),'ellipse sample must be analytic ellipse, not rectangle');await page.getByRole('button',{name:'直线',exact:true}).click();assert.ok(!(await page.locator('.shape-live-sample path').first().getAttribute('d')).includes(' Z'),'line sample must not remain a rectangle');}],
...['none','square','circle','diamond','open_arrow','closed_arrow','butt','r_open_arrow','r_closed_arrow','slash'].map(ending=>{
  let s=existing('line');s.original.line.endings=[ending,ending];s.original.fill={r:1,g:1,b:0};s.original.line_width=32;
  return [`imported-${ending}-style-preservation`,{selections:[s]},async page=>{await activate(page);const b=await box(page);await page.mouse.click(b.x+b.width*270/595,b.y+b.height*(842-370)/842);assert.equal(await page.getByRole('spinbutton',{name:'形状线宽',exact:true}).inputValue(),'32');const f=page.getByRole('spinbutton',{name:'形状透明度',exact:true});await f.fill('50');await f.dispatchEvent('change');await settled(page);const d=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args.draft);assert.equal(d.line_width,32);assert.deepEqual(d.geometry.endings,[ending,ending]);assert.deepEqual(d.geometry.dash_pattern,[3,2]);assert.deepEqual(d.fill,{r:1,g:1,b:0});assert.equal(d.opacity,.5);}];
}),
...['missingRows','gap'].map(key=>[`projection-${key}-refuses`,{[key]:true},async page=>{await activate(page);await drag(page);assert.equal(await count(page,'begin_canvas_shape_editor'),0);}]),
...['id','page','target'].map(wrongOwner=>[`begin-wrong-${wrongOwner}`,{wrongOwner},async page=>{await activate(page);await drag(page);await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('不匹配'));assert.equal(await count(page,'commit_shape_editor'),0);assert.equal(await count(page,'render_shape_editor_background'),0);}]),
...['none','owner','doc'].map(syncConflict=>[`sync-${syncConflict}-conflict`,{commit:'timeout',syncConflict},async page=>{await activate(page);await drag(page);await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('未知'));await page.getByRole('button',{name:'同步形状状态',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('不是完成证明'));assert.equal(await count(page,'commit_shape_editor'),1);}]),
...['Widget','Popup','Stamp'].map(subtype=>[`unsupported-${subtype}`,{rows:[row(subtype,{x:50,y:50,width:500,height:700})]},async page=>{await activate(page);await drag(page);assert.equal(await count(page,'begin_canvas_shape_editor'),0);}]),
['offpage-widget-blank-admitted',{rows:[row('Widget',{x:-100,y:0,width:50,height:30})]},async page=>{await activate(page);await drag(page);await settled(page);assert.equal(await count(page,'commit_shape_editor'),1);}],
['locked-refused',{selections:[existing('rectangle',{flags:128})]},async page=>{await activate(page);await drag(page,[270/595,(842-370)/842],[290/595,(842-390)/842]);assert.equal(await count(page,'begin_canvas_shape_editor'),0);}],
...['confirmed','cancelled'].map(mode=>[`native-delete-${mode}`,{selections:[existing()],deleteCancel:mode==='cancelled'},async page=>{await activate(page);const b=await box(page);await page.mouse.click(b.x+b.width*270/595,b.y+b.height*(842-370)/842);await page.keyboard.press('Delete');await waitCount(page,'delete_shape_editor');assert.equal(await count(page,'commit_shape_editor'),0);if(mode==='confirmed')await settled(page);else{await page.waitForFunction(()=>document.querySelector('.canvas-shapes-recovery')?.textContent.includes('cancelled'));assert.equal(await page.locator('.canvas-shape-handle').count(),4);assert.notEqual(await page.evaluate(()=>window.fixture.owner),null);}}]),
...['矩形','椭圆','直线','单箭头','双箭头'].flatMap((label,i)=>[0,90,180,270].map(rotation=>[`geometry-${i}-${rotation}`,{rotation},async page=>{
  await activate(page);await page.getByRole('button',{name:label,exact:true}).click();const a=i%2?[.65,.55]:[.3,.3],z=i===2?[.65,.3]:[.45,.7];const b=await drag(page,a,z);await settled(page);const d=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_shape_editor').args.draft);
  const source=([x,y])=>rotation===90?[y*595,x*842]:rotation===180?[(1-x)*595,y*842]:rotation===270?[(1-y)*595,(1-x)*842]:[x*595,(1-y)*842];const p=source(a),q=source(z);assert.equal(d.geometry.kind,['rectangle','ellipse','line','line','line'][i]);
  if(i<2){assert.ok(Math.abs(d.geometry.bounds.x-Math.min(p[0],q[0]))<3);assert.ok(Math.abs(d.geometry.bounds.y-Math.min(p[1],q[1]))<3);}else{assert.ok(Math.abs(d.geometry.start.x-p[0])<3);assert.ok(Math.abs(d.geometry.end.y-q[1])<3);assert.notEqual(d.geometry.bounds,null);assert.deepEqual(d.geometry.endings,[i===4?'open_arrow':'none',i>=3?'open_arrow':'none']);}
  assert.equal(await page.locator('.canvas-shape-handle').count(),4);assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
  const geometry=await page.evaluate(()=>{const r=document.querySelector('#pdf-reader'),i=document.querySelector('#pdf-page-image');r.scrollTop=0;const top=i.getBoundingClientRect().top>=r.getBoundingClientRect().top;r.scrollTop=r.scrollHeight;return{top,bottom:i.getBoundingClientRect().bottom<=r.getBoundingClientRect().bottom+1,reader:r.getBoundingClientRect().toJSON(),image:i.getBoundingClientRect().toJSON()};});assert.ok(geometry.top&&geometry.bottom);fs.writeFileSync(path.join(evidence,`geometry-${i}-${rotation}-${page.viewportSize().width}.json`),JSON.stringify({draft:d,geometry},null,2));
}]))
];
(async()=>{
  await new Promise(r=>server.listen(0,'127.0.0.1',r));
  const browser=await chromium.launch({headless:true,executablePath:process.env.PLAYWRIGHT_EXECUTABLE_PATH});
  try { let passed=0; for(const width of [1280,390]) for(const [name,options,run] of cases) {
    if(process.env.SHAPES_CASE&&process.env.SHAPES_CASE!==name)continue;
    const page=await open(browser,width,options);
    try{await run(page);assert.deepEqual(page.errors,[]);assert.deepEqual(page.consoleErrors,[]);assert.deepEqual(page.networkErrors,[]);await page.screenshot({path:path.join(evidence,`${name}-${width}.png`),fullPage:true});passed++;}
    catch(error){await page.screenshot({path:path.join(evidence,`FAIL-${name}-${width}.png`),fullPage:true});fs.writeFileSync(path.join(evidence,`FAIL-${name}-${width}.json`),JSON.stringify(await page.evaluate(()=>({requests:window.requests,fixture:window.fixture,status:document.querySelector('.statusbar')?.textContent})),null,2));console.error(`${name} @${width}; evidence ${evidence}`);throw error;}finally{await page.close();}
  } console.log(`canvas-shapes browser: ${passed} cases passed; console/network errors 0; evidence ${evidence}`);
  }finally{await browser.close();server.close();}
})().catch(e=>{console.error(e);server.close();process.exitCode=1;});
