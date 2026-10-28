// Compiled Rust/WASM UI with synthetic native replies, not OS picker or PDF acceptance.
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
// Read the same output as Trunk.toml / Tauri frontendDist, not stale app/dist.
const root=path.resolve(__dirname,'../dist');
const server=http.createServer((req,res)=>{
  const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);
  if(file!==root&&!file.startsWith(root+path.sep)){res.writeHead(403);return res.end();}
  const target=file===root?path.join(root,'index.html'):file;
  try{res.setHeader('Content-Type',target.endsWith('.wasm')?'application/wasm':target.endsWith('.js')?'text/javascript':target.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(target));}catch{res.writeHead(404);res.end();}
});
const draft={
  id:7,name:'原有草稿.pdf',tabs:[{id:7,name:'原有草稿.pdf',dirty:true}],restored:true,
  info:{page_count:1,pages:[{width:595,height:842,rotation:0}],dirty:true,can_undo:true,can_redo:false},
};
const converted=(id,name)=>({
  id,name,tabs:[{id:7,name:'原有草稿.pdf',dirty:true},{id,name,dirty:true}],restored:false,
  info:{page_count:3,pages:[{width:595,height:842,rotation:0},{width:595,height:842,rotation:0},{width:595,height:842,rotation:0}],dirty:true,can_undo:false,can_redo:false},
});
async function requestCount(page,command){return page.evaluate(c=>window.requests.filter(r=>r.command===c).length,command);}
async function waitForRequestCount(page,command,count){await page.waitForFunction(({command,count})=>window.requests.filter(r=>r.command===command).length>=count,{command,count},{timeout:3000});}
async function assertNoCard(page){
  assert.equal(await page.getByRole('dialog').count(),0,'tool must not open a configuration card');
  assert.equal(await page.locator('.image-import-backdrop').count(),0,'tool must not mount the rejected backdrop');
  assert.equal(await page.getByRole('combobox',{name:'纸型',exact:true}).count(),0,'paper controls must not exist');
  assert.equal(await page.getByRole('combobox',{name:'方向',exact:true}).count(),0,'orientation controls must not exist');
  assert.equal(await page.getByRole('textbox',{name:'边距（pt）',exact:true}).count(),0,'margin controls must not exist');
  assert.equal(await page.getByRole('listbox',{name:'转换顺序 / 来源图片',exact:true}).count(),0,'image reorder list must not exist');
  for(const name of ['上移','下移','移除'])assert.equal(await page.getByRole('button',{name,exact:true}).count(),0,`${name} action must not exist`);
  assert.equal(await page.getByRole('button',{name:'创建新 PDF',exact:true}).count(),0,'extra Create confirmation must not exist');
}
async function queuePicker(page,value){await page.evaluate(value=>window.fixture.pickers.push(value),value);}
async function queueConvert(page,value){await page.evaluate(value=>window.fixture.converts.push(value),value);}
async function queueCancel(page,value){await page.evaluate(value=>window.fixture.cancels.push(value),value);}
async function sendProgress(page,id,payload){await page.evaluate(({id,payload})=>{
  const channel=window.fixture.progress.get(id);if(!channel)throw new Error(`missing progress channel ${id}`);
  channel.onmessage(payload);
},{id,payload});}
async function releaseCancel(page,id,reply){await page.evaluate(({id,reply})=>{
  const pending=window.fixture.cancelPending.get(id);if(!pending)throw new Error(`missing pending cancellation ${id}`);
  window.fixture.cancelPending.delete(id);reply.reject?pending.reject(reply.reject):pending.resolve(reply.value);
},{id,reply});}
async function statusText(page){return page.getByRole('status').last().innerText();}
async function setChannelMode(page,mode){await page.evaluate(mode=>{
  const core=window.__TAURI__.core;
  if(mode==='normal')core.Channel=window.fixture.Channel;
  else if(mode==='missing')delete core.Channel;
  else if(mode==='throw')core.Channel=class{constructor(callback){window.fixture.retainedThrowCallback=callback;throw new Error('synthetic Channel constructor failure');}};
  else throw new Error(`unknown Channel mode ${mode}`);
},mode);}
async function sendRetainedThrowProgress(page,payload){await page.evaluate(payload=>{
  if(!window.fixture.retainedThrowCallback)throw new Error('missing retained throwing-constructor callback');
  window.fixture.retainedThrowCallback(payload);
},payload);}
async function releaseConvert(page,id,reply){await page.evaluate(({id,reply})=>{
  const pending=window.fixture.pending.get(id);if(!pending)throw new Error(`missing pending conversion ${id}`);
  window.fixture.pending.delete(id);reply.ok?pending.resolve(reply.document):pending.reject(reply.error);
},{id,reply});}
async function openTools(page){
  await page.getByRole('button',{name:'主界面标签',exact:true}).click();
  await page.getByRole('button',{name:'工具',exact:true}).click();
  const tool=page.getByRole('button',{name:'图片转 PDF',exact:true});await tool.waitFor({state:'visible'});return tool;
}
(async()=>{
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try{
    browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_EXECUTABLE_PATH?{executablePath:process.env.PLAYWRIGHT_EXECUTABLE_PATH}:{})});
    for(const width of [1280,390]){
      const page=await browser.newPage({viewport:{width,height:900}});page.setDefaultTimeout(3000);
      const pageErrors=[];page.on('pageerror',error=>pageErrors.push(String(error)));
      await page.addInitScript(initialDraft=>{
        class FixtureChannel{constructor(cb){this.onmessage=cb;}}
        window.requests=[];window.fixture={pickers:[],converts:[],cancels:[],pending:new Map(),cancelPending:new Map(),progress:new Map(),plainConvertArgs:[],Channel:FixtureChannel,retainedThrowCallback:null,next:0};
        window.__TAURI__={core:{Channel:FixtureChannel,invoke:async(command,args)=>{
          if(command==='convert_image_import'){
            window.fixture.plainConvertArgs.push(!(args instanceof Map)&&Object.getPrototypeOf(args)===Object.prototype);
            const channel=args instanceof Map?args.get('on_progress'):args&&args.on_progress;
            const id=args instanceof Map?args.get('id'):args&&args.id;
            if(channel)window.fixture.progress.set(id,channel);
          }
          args=JSON.parse(JSON.stringify(args,(_k,v)=>v instanceof Map?Object.fromEntries(v):v));window.requests.push({command,args});
          if(command==='convert_image_import'&&args)delete window.requests.at(-1).args.on_progress;
          if(command==='document_info')return initialDraft;
          if(command==='register_file_drop_channel')return 1;
          if(command==='unregister_file_drop_channel')return null;
          if(command==='prepare_image_import'){
            const next=window.fixture.pickers.length?window.fixture.pickers.shift():{id:++window.fixture.next,names:['first.png','second.tiff','third.jpg']};
            if(next&&next.id>window.fixture.next)window.fixture.next=next.id;return next;
          }
          if(command==='cancel_image_import'){
            const next=window.fixture.cancels.length?window.fixture.cancels.shift():true;
            if(next&&next.pending)return new Promise((resolve,reject)=>window.fixture.cancelPending.set(args.id,{resolve,reject}));
            if(next&&next.reject)throw next.reject;return next;
          }
          if(command==='convert_image_import'){
            const next=window.fixture.converts.length?window.fixture.converts.shift():{pending:true};
            if(next.reject)throw next.reject;if(next.document)return next.document;
            return new Promise((resolve,reject)=>window.fixture.pending.set(args.id,{resolve,reject}));
          }
          if(command==='render_page'||command==='render_overview_page')return 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U0AAAAASUVORK5CYII=';
          if(command==='prepare_page_overview')return 1;
          throw new Error('unexpected fixture command '+command);
        }}};
      },draft);
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.getByRole('button',{name:'文档标签 原有草稿.pdf',exact:true}).waitFor();
      let tool=await openTools(page);assert.equal(await tool.isEnabled(),true,'image tool must work while preserving an existing draft tab');

      await queuePicker(page,null);const prepare0=await requestCount(page,'prepare_image_import');
      await tool.click();await waitForRequestCount(page,'prepare_image_import',prepare0+1);
      await page.waitForFunction(()=>!document.querySelector('[aria-label="图片转 PDF"]').disabled,undefined,{timeout:3000});
      assert.equal(await requestCount(page,'convert_image_import'),0,'dismissed picker must not convert');
      assert.equal(await page.getByRole('button',{name:'文档标签 原有草稿.pdf',exact:true}).count(),1);

      const invalidCancel0=await requestCount(page,'cancel_image_import');
      await queueCancel(page,false);await queuePicker(page,{id:0,names:['invalid.png']});await tool.click();
      await waitForRequestCount(page,'cancel_image_import',invalidCancel0+1);
      assert.equal(await requestCount(page,'convert_image_import'),0,'invalid ID 0 must never convert');
      assert.equal(await tool.isDisabled(),true,'unconfirmed invalid-selection cleanup must keep tools blocked');
      let cancel=page.getByRole('button',{name:'取消图片转换',exact:true});await cancel.waitFor();
      assert.ok((await page.getByRole('status').last().innerText()).includes('取消未确认'));
      assert.deepEqual((await page.evaluate(()=>window.requests.findLast(r=>r.command==='cancel_image_import'))).args,{id:0},'invalid selection recovery must retain the native ID');
      await queueCancel(page,true);await cancel.click();await cancel.waitFor({state:'hidden'});
      await page.waitForFunction(()=>!document.querySelector('[aria-label="图片转 PDF"]').disabled,undefined,{timeout:3000});

      const invalidCancel1=await requestCount(page,'cancel_image_import');
      await queueCancel(page,{reject:'synthetic invalid cleanup transport error'});await queuePicker(page,{id:90,names:['bad/name.png']});await tool.click();
      await waitForRequestCount(page,'cancel_image_import',invalidCancel1+1);
      assert.equal(await requestCount(page,'convert_image_import'),0,'invalid names must never convert');
      cancel=page.getByRole('button',{name:'取消图片转换',exact:true});await cancel.waitFor();
      assert.ok((await page.getByRole('status').last().innerText()).includes('synthetic invalid cleanup transport error'));
      assert.deepEqual((await page.evaluate(()=>window.requests.findLast(r=>r.command==='cancel_image_import'))).args,{id:90});
      await queueCancel(page,true);await cancel.click();await cancel.waitFor({state:'hidden'});
      await page.waitForFunction(()=>!document.querySelector('[aria-label="图片转 PDF"]').disabled,undefined,{timeout:3000});

      const invalidCancel2=await requestCount(page,'cancel_image_import');
      await queueCancel(page,true);await queuePicker(page,{id:91,names:[]});await tool.click();
      await waitForRequestCount(page,'cancel_image_import',invalidCancel2+1);
      assert.equal(await requestCount(page,'convert_image_import'),0,'empty invalid selection must never convert');
      assert.equal(await page.getByRole('button',{name:'取消图片转换',exact:true}).count(),0,'confirmed invalid cleanup must clear the recovery job');
      await page.waitForFunction(()=>!document.querySelector('[aria-label="图片转 PDF"]').disabled,undefined,{timeout:3000});

      await queuePicker(page,{id:1,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      const prepare1=await requestCount(page,'prepare_image_import');await tool.click();await waitForRequestCount(page,'prepare_image_import',prepare1+1);
      await page.waitForTimeout(100);await assertNoCard(page);
      assert.equal(await requestCount(page,'convert_image_import'),1,'selection must convert without another click');
      assert.equal(await page.evaluate(()=>window.fixture.plainConvertArgs[0]),true,'observed conversion arguments must be a plain object');
      assert.equal(await page.evaluate(()=>window.fixture.progress.get(1) instanceof window.fixture.Channel),true,'conversion must attach the real Channel object before request recording');
      assert.deepEqual((await page.evaluate(()=>window.requests.findLast(r=>r.command==='convert_image_import'))).args,{id:1,order:[0,1,2],layout:{paper:'source',landscape:false,margin:0}});
      assert.equal(await tool.isDisabled(),true,'tool stays disabled while conversion owns the selection');
      assert.ok((await statusText(page)).includes('正在转换图片'),'首个真实事件前保留通用转换状态');
      await sendProgress(page,1,'bad progress payload');
      assert.ok((await statusText(page)).includes('正在转换图片'),'坏格式通知不得改变状态');
      await sendProgress(page,1,{id:1,current:1,total:3});
      await page.waitForFunction(()=>[...document.querySelectorAll('[role=status]')].some(e=>e.textContent.includes('正在处理第 1 张图 ，共 3 张...')));
      let progressStatus=await statusText(page);
      for(const payload of [
        {id:2,current:2,total:3},{id:1,current:2,total:2},{id:1,current:1,total:0},
        {id:1,current:1,total:33},{id:1,current:0,total:3},{id:1,current:4,total:3},
        {id:1,current:1,total:3},
      ]){
        await sendProgress(page,1,payload);
        assert.equal(await statusText(page),progressStatus,'非法或重复进度不得改写计数');
      }
      await sendProgress(page,1,{id:1,current:2,total:3});
      await page.waitForFunction(()=>[...document.querySelectorAll('[role=status]')].some(e=>e.textContent.includes('正在处理第 2 张图 ，共 3 张...')));
      progressStatus=await statusText(page);
      await sendProgress(page,1,{id:1,current:1,total:3});
      assert.equal(await statusText(page),progressStatus,'回退进度不得改写计数');
      await sendProgress(page,1,{id:1,current:3,total:3});
      await page.waitForFunction(()=>[...document.querySelectorAll('[role=status]')].some(e=>e.textContent.includes('正在处理第 3 张图 ，共 3 张...')));
      assert.equal(await page.getByRole('button',{name:/文档标签/}).count(),1,'最后一张源图事件不是转换完成');
      await assertNoCard(page);
      cancel=page.getByRole('button',{name:'取消图片转换',exact:true});await cancel.waitFor();
      assert.equal(await page.locator('.home-status button[aria-label="取消图片转换"]').count(),1,'only the Home footer owns the image action');
      assert.equal(await page.locator('.statusbar button[aria-label="取消图片转换"]').count(),0,'Reader must not own an unreachable image action');
      assert.equal(await page.locator('#reader-workspace').isVisible(),false,'Reader remains hidden during the Home-started job');
      const cancelBox=await cancel.boundingBox();
      assert.ok(cancelBox&&cancelBox.width>0&&cancelBox.height>0,'cancel action must remain visible and reachable');
      if(width===390)assert.ok(cancelBox.height>=44,`mobile cancel target must be at least 44px high, got ${cancelBox.height}`);
      await page.screenshot({path:`/tmp/pdfmaster-image-import-no-card-pending-${width}.png`});

      const automaticCancel0=await requestCount(page,'cancel_image_import');await queueCancel(page,false);
      await releaseConvert(page,1,{ok:false,error:'synthetic conversion failure'});await waitForRequestCount(page,'cancel_image_import',automaticCancel0+1);
      await page.waitForFunction(()=>!document.querySelector('[aria-label="取消图片转换"]').disabled,undefined,{timeout:3000});
      assert.equal(await tool.isDisabled(),true,'automatic cleanup false must keep tools blocked');
      assert.equal(await page.locator('#reader-workspace').isVisible(),false,'Reader remains hidden while automatic cleanup is unconfirmed');
      assert.ok((await page.getByRole('status').last().innerText()).includes('取消未确认'));
      const failedStatus=await statusText(page);await sendProgress(page,1,{id:1,current:3,total:3});
      assert.equal(await statusText(page),failedStatus,'settled conversion progress callback must be inert during error cleanup');
      await queueCancel(page,{reject:'synthetic automatic cleanup transport error'});await cancel.click();
      await page.waitForFunction(()=>!document.querySelector('[aria-label="取消图片转换"]').disabled,undefined,{timeout:3000});
      assert.ok((await page.getByRole('status').last().innerText()).includes('synthetic automatic cleanup transport error'));
      await queueCancel(page,true);await cancel.click();await cancel.waitFor({state:'hidden'});
      await page.waitForFunction(()=>!document.querySelector('[aria-label="图片转 PDF"]').disabled,undefined,{timeout:3000});
      await assertNoCard(page);assert.equal(await page.getByRole('button',{name:/文档标签/}).count(),1,'failure must not create a tab');
      assert.equal(await page.getByRole('button',{name:'文档标签 原有草稿.pdf',exact:true}).count(),1,'failure must preserve the draft tab');
      assert.ok((await page.getByRole('status').last().innerText()).includes('synthetic conversion failure'));
      await page.screenshot({path:`/tmp/pdfmaster-image-import-no-card-failure-${width}.png`});

      await queuePicker(page,{id:2,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',2);cancel=page.getByRole('button',{name:'取消图片转换',exact:true});await cancel.waitFor();
      assert.ok((await statusText(page)).includes('正在转换图片'),'new conversion must not reuse the previous source count');
      await sendProgress(page,1,{id:1,current:3,total:3});
      assert.ok((await statusText(page)).includes('正在转换图片'),'old settled channel must not affect a new conversion');
      await sendProgress(page,2,{id:2,current:1,total:3});
      await page.waitForFunction(()=>[...document.querySelectorAll('[role=status]')].some(e=>e.textContent.includes('正在处理第 1 张图 ，共 3 张...')));
      const automaticCancel1=await requestCount(page,'cancel_image_import');await queueCancel(page,{reject:'synthetic automatic cleanup transport error'});
      await releaseConvert(page,2,{ok:false,error:'synthetic second conversion failure'});await waitForRequestCount(page,'cancel_image_import',automaticCancel1+1);
      await page.waitForFunction(()=>!document.querySelector('[aria-label="取消图片转换"]').disabled,undefined,{timeout:3000});
      assert.equal(await tool.isDisabled(),true,'automatic cleanup transport error must keep tools blocked');
      assert.ok((await page.getByRole('status').last().innerText()).includes('synthetic automatic cleanup transport error'));
      const transportErrorStatus=await statusText(page);await sendProgress(page,2,{id:2,current:3,total:3});
      assert.equal(await statusText(page),transportErrorStatus,'settled progress must not overwrite cleanup transport error');
      await queueCancel(page,true);await cancel.click();await cancel.waitFor({state:'hidden'});
      await page.waitForFunction(()=>!document.querySelector('[aria-label="图片转 PDF"]').disabled,undefined,{timeout:3000});

      await queuePicker(page,{id:3,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',3);cancel=page.getByRole('button',{name:'取消图片转换',exact:true});await cancel.waitFor();
      await sendProgress(page,3,{id:3,current:1,total:3});
      await page.waitForFunction(()=>[...document.querySelectorAll('[role=status]')].some(e=>e.textContent.includes('正在处理第 1 张图 ，共 3 张...')));
      const cancelRequest0=await requestCount(page,'cancel_image_import');await queueCancel(page,{pending:true});await cancel.click();
      await waitForRequestCount(page,'cancel_image_import',cancelRequest0+1);
      assert.ok((await statusText(page)).includes('正在取消图片转换'),'pending cancellation must outrank source progress');
      const pendingCancelStatus=await statusText(page);await sendProgress(page,3,{id:3,current:2,total:3});
      assert.equal(await statusText(page),pendingCancelStatus,'progress must not overwrite pending cancellation');
      await releaseCancel(page,3,{value:false});await page.waitForFunction(()=>!document.querySelector('[aria-label="取消图片转换"]').disabled,undefined,{timeout:3000});
      assert.ok((await statusText(page)).includes('取消未确认'));
      const falseCancelStatus=await statusText(page);await sendProgress(page,3,{id:3,current:3,total:3});
      assert.equal(await statusText(page),falseCancelStatus,'progress must not overwrite a false cancellation result');
      await queueCancel(page,{reject:'synthetic cancel transport error'});await cancel.click();await page.waitForFunction(()=>!document.querySelector('[aria-label="取消图片转换"]').disabled,undefined,{timeout:3000});
      assert.ok((await page.getByRole('status').last().innerText()).includes('synthetic cancel transport error'));
      const rejectedCancelStatus=await statusText(page);await sendProgress(page,3,{id:3,current:3,total:3});
      assert.equal(await statusText(page),rejectedCancelStatus,'progress must not overwrite a cancellation transport error');
      await queueCancel(page,true);await cancel.click();await cancel.waitFor({state:'hidden'});
      assert.equal(await page.getByRole('button',{name:/文档标签/}).count(),1,'confirmed cancellation must not create a tab');

      await queuePicker(page,{id:4,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',4);
      assert.ok((await statusText(page)).includes('正在转换图片'),'new job begins without an old source count');
      await sendProgress(page,3,{id:3,current:3,total:3});
      assert.ok((await statusText(page)).includes('正在转换图片'),'unsettled old job progress must not affect current job');
      await releaseConvert(page,3,{ok:false,error:'late old rejection'});await page.waitForTimeout(100);
      await sendProgress(page,3,{id:3,current:3,total:3});
      assert.ok((await statusText(page)).includes('正在转换图片'),'settled old job channel must be inert');
      assert.equal(await page.getByRole('button',{name:'取消图片转换',exact:true}).count(),1,'old rejection must not clear the new job');assert.equal(await tool.isDisabled(),true);
      await queueCancel(page,true);await page.getByRole('button',{name:'取消图片转换',exact:true}).click();await page.getByRole('button',{name:'取消图片转换',exact:true}).waitFor({state:'hidden'});

      await queuePicker(page,{id:5,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',5);await sendProgress(page,4,{id:4,current:1,total:3});
      assert.ok((await statusText(page)).includes('正在转换图片'),'old job progress must not cross into a newer task');
      await releaseConvert(page,4,{ok:true,document:converted(104,'迟到旧文档.pdf')});await page.waitForTimeout(100);
      await sendProgress(page,4,{id:4,current:2,total:3});
      assert.equal(await page.getByRole('button',{name:'文档标签 迟到旧文档.pdf',exact:true}).count(),0,'stale success must not publish');
      assert.equal(await page.getByRole('button',{name:'取消图片转换',exact:true}).count(),1,'stale success must not clear current job');
      await queueCancel(page,true);await page.getByRole('button',{name:'取消图片转换',exact:true}).click();await page.getByRole('button',{name:'取消图片转换',exact:true}).waitFor({state:'hidden'});

      await setChannelMode(page,'missing');
      await queuePicker(page,{id:6,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{document:converted(106,'无通道降级.pdf')});
      const convert6=await requestCount(page,'convert_image_import');await tool.click();await waitForRequestCount(page,'convert_image_import',convert6+1);
      await page.getByRole('button',{name:'文档标签 无通道降级.pdf',exact:true}).waitFor();await page.waitForTimeout(100);
      assert.equal(await requestCount(page,'convert_image_import'),convert6+1,'missing Channel constructor must invoke conversion exactly once');
      assert.equal(await page.getByRole('button',{name:'文档标签 原有草稿.pdf',exact:true}).count(),1);await assertNoCard(page);
      await setChannelMode(page,'normal');tool=await openTools(page);

      await setChannelMode(page,'throw');
      await queuePicker(page,{id:7,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      const convert7=await requestCount(page,'convert_image_import');await tool.click();await waitForRequestCount(page,'convert_image_import',convert7+1);
      assert.ok((await statusText(page)).includes('正在转换图片'),'throw fallback stays generic before retained callback');
      await sendRetainedThrowProgress(page,{id:7,current:1,total:3});await page.waitForTimeout(50);
      assert.ok((await statusText(page)).includes('正在转换图片'),'callback retained by throwing constructor must already be inert');
      assert.equal(await requestCount(page,'convert_image_import'),convert7+1,'throwing Channel constructor must invoke conversion exactly once');
      assert.equal(await page.getByRole('button',{name:'文档标签 通道异常降级.pdf',exact:true}).count(),0,'retained callback must not publish its document');await assertNoCard(page);
      await releaseConvert(page,7,{ok:true,document:converted(107,'通道异常降级.pdf')});
      await page.getByRole('button',{name:'文档标签 通道异常降级.pdf',exact:true}).waitFor();await page.waitForTimeout(100);
      const settledThrowStatus=await statusText(page);await sendRetainedThrowProgress(page,{id:7,current:2,total:3});await page.waitForTimeout(50);
      assert.equal(await statusText(page),settledThrowStatus,'retained throwing-constructor callback must stay inert after settle');
      assert.equal(await requestCount(page,'convert_image_import'),convert7+1,'settled throw fallback must remain single-invoke');
      assert.equal(await page.getByRole('button',{name:'文档标签 原有草稿.pdf',exact:true}).count(),1);await assertNoCard(page);
      assert.equal(pageErrors.length,0,`retained throwing-constructor callback caused page errors: ${pageErrors.join('; ')}`);
      await setChannelMode(page,'normal');tool=await openTools(page);

      await queuePicker(page,{id:8,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',8);const renders=await requestCount(page,'render_page');
      assert.ok((await statusText(page)).includes('正在转换图片'),'new task must remain generic before its first event');
      await sendProgress(page,5,{id:5,current:1,total:3});
      assert.ok((await statusText(page)).includes('正在转换图片'),'unsettled cancelled job progress must not affect final job');
      await sendProgress(page,8,{id:8,current:1,total:3});
      await page.waitForFunction(()=>[...document.querySelectorAll('[role=status]')].some(e=>e.textContent.includes('正在处理第 1 张图 ，共 3 张...')));
      await sendProgress(page,8,{id:8,current:3,total:3});
      assert.equal(await page.getByRole('button',{name:'文档标签 图片合并.pdf',exact:true}).count(),0,'final source event must not publish a document');
      await assertNoCard(page);
      await releaseConvert(page,8,{ok:true,document:converted(108,'图片合并.pdf')});await waitForRequestCount(page,'render_page',renders+1);
      await page.getByRole('button',{name:'文档标签 图片合并.pdf',exact:true}).waitFor();
      assert.equal(await page.getByRole('button',{name:'文档标签 原有草稿.pdf',exact:true}).count(),1,'success keeps the pre-existing draft tab');
      assert.equal(await page.locator('#reader-workspace').isVisible(),true);await assertNoCard(page);
      await page.screenshot({path:`/tmp/pdfmaster-image-import-no-card-success-${width}.png`});
      await sendProgress(page,8,{id:8,current:3,total:3});
      assert.equal(await page.getByRole('button',{name:'文档标签 图片合并.pdf',exact:true}).count(),1,'settled current channel callback must be inert');
      await releaseConvert(page,5,{ok:false,error:'late cancelled job rejection'});await page.waitForTimeout(100);
      assert.equal(await page.getByRole('button',{name:'文档标签 图片合并.pdf',exact:true}).count(),1,'late rejection cannot remove the published current result');
      await sendProgress(page,5,{id:5,current:2,total:3});await page.waitForTimeout(50);
      assert.equal(pageErrors.length,0,`late/detached channel callbacks caused page errors: ${pageErrors.join('; ')}`);
      await page.close();console.log(`image no-card replay ${width}: passed (compiled WASM, synthetic native replies)`);
    }
  }finally{if(browser)await browser.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
