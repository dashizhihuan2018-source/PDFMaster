// Compiled Rust/WASM UI with synthetic native replies; not native-runtime acceptance.
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const root=path.resolve(__dirname,'../dist');
const image='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U0AAAAASUVORK5CYII=';
const server=http.createServer((req,res)=>{
  const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);
  if(file!==root&&!file.startsWith(root+path.sep)){res.writeHead(403);return res.end();}
  const target=file===root?path.join(root,'index.html'):file;
  try{res.setHeader('Content-Type',target.endsWith('.wasm')?'application/wasm':target.endsWith('.js')?'text/javascript':target.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(target));}catch{res.writeHead(404);res.end();}
});
function initialDocument(inkEditor=null){return{
  id:7,name:'synthetic.pdf',tabs:[{id:7,name:'synthetic.pdf',dirty:false}],restored:false,
  info:{page_count:1,pages:[{width:595,height:842,rotation:0}],dirty:false,can_undo:false,can_redo:false},
  ...(inkEditor?{ink_editor:inkEditor}:{}),
};}
async function requestCount(page,command){return page.evaluate(command=>window.requests.filter(r=>r.command===command).length,command);}
async function waitForCount(page,command,count){
  try{await page.waitForFunction(({command,count})=>window.requests.filter(r=>r.command===command).length>=count,{command,count});}
  catch(error){const state=await page.evaluate(()=>({requests:window.requests,status:[...document.querySelectorAll('[role=status]')].map(node=>node.textContent),preview:document.querySelector('.ink-preview')?.outerHTML||null,pointerEvents:window.fixture.pointerEvents,surface:(()=>{const r=document.querySelector('.drawing-surface')?.getBoundingClientRect();return r&&{x:r.x,y:r.y,width:r.width,height:r.height};})()}));throw new Error(`${command} did not reach ${count}: ${JSON.stringify(state)}\n${error.message}`);}
}
async function idle(page){await page.waitForFunction(()=>document.querySelector('#pdf-reader')?.getAttribute('aria-busy')==='false');}
async function setMode(page,key,value){await page.evaluate(({key,value})=>{window.fixture[key]=value;},{key,value});}
async function release(page,key,reply){await page.evaluate(({key,reply})=>{
  const pending=window.fixture[key];if(!pending)throw new Error(`missing pending ${key}`);window.fixture[key]=null;
  if(reply.reject)pending.reject(reply.reject);else pending.resolve(reply.value);
},{key,reply});}
async function activateSecondaryPen(page){
  await page.getByRole('button',{name:'批注',exact:true}).click();
  const pen=page.getByRole('button',{name:'工具 · 手绘',exact:true});await pen.click();return pen;
}
async function assertDirectInk(page,pen){
  assert.equal(await page.locator('.drawing-surface.inking').count(),1,'pen activation must directly enable the drawing surface');
  assert.equal(await page.locator('.inspector:visible').count(),0,'pen activation must not show the parameter inspector');
  assert.equal(await page.locator('#tool-ink').count(),0,'Ink parameter form must not remain mounted');
  assert.equal(await page.getByRole('button',{name:'进入手绘模式',exact:true}).count(),0,'second activation button must not exist');
  assert.equal(await page.getByRole('textbox',{name:'笔迹颜色',exact:true}).count(),0,'Ink color input must not exist');
  assert.equal(await page.getByRole('group',{name:'手绘事务恢复',exact:true}).count(),0,'idle Ink must not show recovery actions');
  assert.equal(await pen.getAttribute('aria-pressed'),'true');
}
async function drawStroke(page){
  const box=await page.locator('.drawing-surface').boundingBox();assert.ok(box&&box.width>20&&box.height>20,'drawing surface must have geometry');
  const x=box.x+Math.min(40,box.width*.2),y=box.y+Math.min(40,box.height*.2);
  const dx=Math.min(120,box.width*.45),dy=Math.min(140,box.height*.35);
  await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x+dx,y+dy,{steps:8});await page.mouse.up();
}
async function newPage(browser,width,{inkEditor=null}={}){
  const page=await browser.newPage({viewport:{width,height:900}});page.setDefaultTimeout(4000);
  const pageErrors=[];page.on('pageerror',error=>pageErrors.push(String(error)));
  await page.addInitScript(({document,image})=>{
    const clone=value=>JSON.parse(JSON.stringify(value,(_key,item)=>item instanceof Map?Object.fromEntries(item):item));
    window.requests=[];window.fixture={document,render:0,nextInk:20,beginMode:'resolve',commitMode:'resolve',cancelMode:'resolve',pendingBegin:null,pendingCommit:null,pendingCancel:null,pointerEvents:[]};
    for(const type of ['pointerdown','pointermove','pointerup'])window.addEventListener(type,event=>{if(event.target.closest?.('.drawing-surface')){const r=globalThis.document.querySelector('.drawing-surface').getBoundingClientRect();window.fixture.pointerEvents.push({type,x:event.clientX,y:event.clientY,pointer:event.pointerId,target:event.target.className,rect:[r.x,r.y,r.width,r.height]});}},true);
    const nativeDocument=()=>{const doc=clone(window.fixture.document);doc.tabs=[{id:doc.id,name:doc.name,dirty:doc.info.dirty}];return doc;};
    const finishInk=()=>{delete window.fixture.document.ink_editor;window.fixture.document.info.dirty=true;window.fixture.document.info.can_undo=true;window.fixture.render++;return nativeDocument();};
    window.__TAURI__={core:{invoke:async(command,args)=>{
      args=args instanceof Map?Object.fromEntries(args):args;args=clone(args||{});window.requests.push({command,args});
      if(command==='document_info')return nativeDocument();
      if(command==='render_page'||command==='render_overview_page')return `${image}#render-${window.fixture.render}`;
      if(command==='notes_document')return {notes:[],unsupported_count:0};
      if(command==='shapes_document')return {shapes:[],unsupported_count:0};
      if(command==='annotations_document')return [];
      if(command==='search_document')return {query:args.query,case_sensitive:false,hits:[],pages_scanned:1,searchable_pages:1,truncated:false,limitation:''};
      if(command==='prepare_page_overview')return 1;
      if(command==='begin_ink_document'){
        const editor={id:window.fixture.nextInk++,page:args.page,request:args.request};
        if(window.fixture.beginMode==='reject'){window.fixture.beginMode='resolve';throw new Error('synthetic begin failure');}
        if(window.fixture.beginMode==='pending')return new Promise((resolve,reject)=>{window.fixture.pendingBegin={resolve:value=>{window.fixture.document.ink_editor=clone(value);resolve(value);},reject};});
        window.fixture.document.ink_editor=clone(editor);return editor;
      }
      if(command==='commit_ink_document'){
        if(window.fixture.commitMode==='reject'){window.fixture.commitMode='resolve';throw new Error('synthetic commit failure');}
        if(window.fixture.commitMode==='pending')return new Promise((resolve,reject)=>{window.fixture.pendingCommit={resolve:()=>resolve(finishInk()),reject};});
        return finishInk();
      }
      if(command==='cancel_ink_document'||command==='cancel_ink_request_document'){
        if(window.fixture.cancelMode==='reject'){window.fixture.cancelMode='resolve';throw new Error('synthetic cancel failure');}
        if(window.fixture.cancelMode==='pending')return new Promise((resolve,reject)=>{window.fixture.pendingCancel={resolve:()=>resolve(finishInk()),reject};});
        return finishInk();
      }
      throw new Error('unexpected fixture command '+command);
    }}};
  },{document:initialDocument(inkEditor),image});
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.getByRole('button',{name:'文档标签 synthetic.pdf',exact:true}).waitFor();await idle(page);
  await page.locator('#pdf-page-image').waitFor();await page.waitForFunction(()=>document.querySelector('#pdf-page-image')?.complete);
  return {page,pageErrors};
}
async function assertNoPageErrors(pageErrors,label){assert.deepEqual(pageErrors,[],`${label}: unexpected page errors`);}
(async()=>{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
  try{
    browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_EXECUTABLE_PATH?{executablePath:process.env.PLAYWRIGHT_EXECUTABLE_PATH}:{})});
    for(const width of [1280,390]){
      const {page,pageErrors}=await newPage(browser,width);
      const pen=await activateSecondaryPen(page);await assertDirectInk(page,pen);
      await pen.click();assert.equal(await page.locator('.drawing-surface.inking').count(),0);assert.equal(await pen.getAttribute('aria-pressed'),'false');

      await setMode(page,'beginMode','pending');await pen.click();await drawStroke(page);
      assert.equal(await requestCount(page,'begin_ink_document'),1);assert.equal(await requestCount(page,'commit_ink_document'),0,'pointer-up before begin ack must wait');
      const beginRequest=await page.evaluate(()=>window.requests.find(r=>r.command==='begin_ink_document'));
      assert.match(beginRequest.args.request,/^[0-9a-f]{32}$/);assert.equal(beginRequest.args.page,0);
      for(const channel of ['r','g','b'])assert.equal(typeof beginRequest.args.color[channel],'number',`begin color ${channel} must use the native wire shape`);
      await release(page,'pendingBegin',{value:{id:20,page:0,request:beginRequest.args.request}});await waitForCount(page,'commit_ink_document',1);await idle(page);
      const commit=await page.evaluate(()=>window.requests.find(r=>r.command==='commit_ink_document'));
      assert.equal(commit.args.id,20);assert.equal(commit.args.strokes.length,1);assert.ok(commit.args.strokes[0].length>=2,'real pointer input must produce nonempty points');
      assert.equal(await requestCount(page,'begin_ink_document'),1);assert.equal(await requestCount(page,'commit_ink_document'),1,'delayed ack must commit exactly once');

      await pen.click();await page.getByRole('button',{name:'编辑',exact:true}).click();assert.equal(await page.locator('.drawing-surface.inking').count(),0,'mode switch clears idle pen');
      await page.getByRole('button',{name:'批注',exact:true}).click();await pen.click();await page.locator('.chrome-search').focus();assert.equal(await page.locator('.drawing-surface.inking').count(),0,'search clears idle pen');
      await pen.click();await page.getByRole('button',{name:'页面预览',exact:true}).click();assert.equal(await page.locator('.drawing-surface.inking').count(),0,'overview clears idle pen');
      await page.getByRole('button',{name:'页面预览',exact:true}).click();

      await page.getByRole('button',{name:'主界面标签',exact:true}).click();await page.getByRole('button',{name:'工具',exact:true}).click();await page.getByRole('button',{name:'绘图',exact:true}).click();
      assert.equal(await page.locator('.drawing-surface.inking').count(),1,'home Draw must activate Ink directly');assert.equal(await page.locator('.inspector:visible').count(),0);
      await pen.click();
      if(width===390&&await page.getByRole('button',{name:'侧栏',exact:true}).getAttribute('aria-pressed')==='false')await page.getByRole('button',{name:'侧栏',exact:true}).click();
      await page.getByRole('button',{name:'侧栏批注',exact:true}).click();const emptyPen=page.getByRole('button',{name:'批注工具 · 画笔',exact:true});await emptyPen.waitFor();await emptyPen.click();
      assert.equal(await page.locator('.drawing-surface.inking').count(),1,'annotation empty-state pen must activate Ink directly');assert.equal(await page.locator('.inspector:visible').count(),0);
      await emptyPen.click();
      await page.locator('summary[aria-label="更多工具"]').click();await page.getByRole('button',{name:'页面管理',exact:true}).click();
      assert.equal(await page.getByRole('button',{name:'工具 · 旋转',exact:true}).getAttribute('aria-pressed'),null);
      await assertNoPageErrors(pageErrors,`core-${width}`);console.log(JSON.stringify({case:'core-direct-ink',width,result:'pass'}));await page.close();
    }

    {
      const {page,pageErrors}=await newPage(browser,1280);const pen=await activateSecondaryPen(page);
      await setMode(page,'beginMode','reject');await drawStroke(page);await waitForCount(page,'begin_ink_document',1);
      const recovery=page.getByRole('group',{name:'手绘事务恢复',exact:true});await recovery.waitFor();
      assert.equal(await recovery.getByRole('button',{name:'重试提交手绘',exact:true}).isDisabled(),true);assert.equal(await recovery.getByRole('button',{name:'取消／同步手绘',exact:true}).isEnabled(),true);
      await recovery.getByRole('button',{name:'取消／同步手绘',exact:true}).click();await waitForCount(page,'cancel_ink_request_document',1);await idle(page);assert.equal(await recovery.count(),0);

      await setMode(page,'commitMode','reject');await drawStroke(page);await waitForCount(page,'commit_ink_document',1);await recovery.waitFor();
      const retry=recovery.getByRole('button',{name:'重试提交手绘',exact:true});assert.equal(await retry.isEnabled(),true);await retry.click();await waitForCount(page,'commit_ink_document',2);await idle(page);assert.equal(await recovery.count(),0,'real retry button clears successful commit');

      await setMode(page,'commitMode','reject');await drawStroke(page);await waitForCount(page,'commit_ink_document',3);await recovery.waitFor();
      await setMode(page,'cancelMode','pending');await recovery.getByRole('button',{name:'取消／同步手绘',exact:true}).click();await waitForCount(page,'cancel_ink_document',1);
      assert.equal(await recovery.count(),1,'busy recovery stays visible');assert.equal(await retry.isDisabled(),true);assert.equal(await recovery.getByRole('button',{name:'取消／同步手绘',exact:true}).isDisabled(),true);
      await release(page,'pendingCancel',{value:null});await idle(page);assert.equal(await recovery.count(),0);
      assert.equal(await pen.getAttribute('aria-pressed'),'true','transaction recovery does not silently change the selected tool');
      await assertNoPageErrors(pageErrors,'failures');console.log(JSON.stringify({case:'begin-commit-recovery',result:'pass'}));await page.close();
    }

    {
      const restored={id:77,page:0,request:'restored-native-request'};const {page,pageErrors}=await newPage(browser,1280,{inkEditor:restored});
      const pen=page.getByRole('button',{name:'工具 · 手绘',exact:true});assert.equal(await pen.getAttribute('aria-pressed'),'true');assert.equal(await page.locator('.inspector:visible').count(),0);
      const recovery=page.getByRole('group',{name:'手绘事务恢复',exact:true});await recovery.waitFor();const cancel=recovery.getByRole('button',{name:'取消／同步手绘',exact:true});
      assert.equal(await recovery.getByRole('button',{name:'重试提交手绘',exact:true}).isDisabled(),true);assert.equal(await cancel.isEnabled(),true);
      await setMode(page,'cancelMode','reject');await cancel.click();await waitForCount(page,'cancel_ink_request_document',1);await idle(page);assert.equal(await recovery.count(),1,'failed native cancellation preserves recovery');assert.equal(await cancel.isEnabled(),true);
      await cancel.click();await waitForCount(page,'cancel_ink_request_document',2);await idle(page);assert.equal(await recovery.count(),0,'successful second cancellation clears restored recovery');
      await assertNoPageErrors(pageErrors,'native-restore');console.log(JSON.stringify({case:'native-restored-recovery',result:'pass'}));await page.close();
    }
  }finally{if(browser)await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error.stack||error.message);process.exitCode=1;});
