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
      await page.addInitScript(initialDraft=>{
        window.requests=[];window.fixture={pickers:[],converts:[],cancels:[],pending:new Map(),next:0};
        window.__TAURI__={core:{Channel:class{constructor(cb){this.onmessage=cb;}},invoke:async(command,args)=>{
          args=JSON.parse(JSON.stringify(args,(_k,v)=>v instanceof Map?Object.fromEntries(v):v));window.requests.push({command,args});
          if(command==='document_info')return initialDraft;
          if(command==='register_file_drop_channel')return 1;
          if(command==='unregister_file_drop_channel')return null;
          if(command==='prepare_image_import'){
            const next=window.fixture.pickers.length?window.fixture.pickers.shift():{id:++window.fixture.next,names:['first.png','second.tiff','third.jpg']};
            if(next&&next.id>window.fixture.next)window.fixture.next=next.id;return next;
          }
          if(command==='cancel_image_import'){
            const next=window.fixture.cancels.length?window.fixture.cancels.shift():true;
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
      const tool=await openTools(page);assert.equal(await tool.isEnabled(),true,'image tool must work while preserving an existing draft tab');

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
      assert.deepEqual((await page.evaluate(()=>window.requests.findLast(r=>r.command==='convert_image_import'))).args,{id:1,order:[0,1,2],layout:{paper:'source',landscape:false,margin:0}});
      assert.equal(await tool.isDisabled(),true,'tool stays disabled while conversion owns the selection');
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
      const automaticCancel1=await requestCount(page,'cancel_image_import');await queueCancel(page,{reject:'synthetic automatic cleanup transport error'});
      await releaseConvert(page,2,{ok:false,error:'synthetic second conversion failure'});await waitForRequestCount(page,'cancel_image_import',automaticCancel1+1);
      await page.waitForFunction(()=>!document.querySelector('[aria-label="取消图片转换"]').disabled,undefined,{timeout:3000});
      assert.equal(await tool.isDisabled(),true,'automatic cleanup transport error must keep tools blocked');
      assert.ok((await page.getByRole('status').last().innerText()).includes('synthetic automatic cleanup transport error'));
      await queueCancel(page,true);await cancel.click();await cancel.waitFor({state:'hidden'});
      await page.waitForFunction(()=>!document.querySelector('[aria-label="图片转 PDF"]').disabled,undefined,{timeout:3000});

      await queuePicker(page,{id:3,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',3);cancel=page.getByRole('button',{name:'取消图片转换',exact:true});await cancel.waitFor();
      await queueCancel(page,false);await cancel.click();await page.waitForFunction(()=>!document.querySelector('[aria-label="取消图片转换"]').disabled,undefined,{timeout:3000});
      assert.ok((await page.getByRole('status').last().innerText()).includes('取消未确认'));
      await queueCancel(page,{reject:'synthetic cancel transport error'});await cancel.click();await page.waitForFunction(()=>!document.querySelector('[aria-label="取消图片转换"]').disabled,undefined,{timeout:3000});
      assert.ok((await page.getByRole('status').last().innerText()).includes('synthetic cancel transport error'));
      await queueCancel(page,true);await cancel.click();await cancel.waitFor({state:'hidden'});
      assert.equal(await page.getByRole('button',{name:/文档标签/}).count(),1,'confirmed cancellation must not create a tab');

      await queuePicker(page,{id:4,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',4);await releaseConvert(page,3,{ok:false,error:'late old rejection'});await page.waitForTimeout(100);
      assert.equal(await page.getByRole('button',{name:'取消图片转换',exact:true}).count(),1,'old rejection must not clear the new job');assert.equal(await tool.isDisabled(),true);
      await queueCancel(page,true);await page.getByRole('button',{name:'取消图片转换',exact:true}).click();await page.getByRole('button',{name:'取消图片转换',exact:true}).waitFor({state:'hidden'});

      await queuePicker(page,{id:5,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',5);await releaseConvert(page,4,{ok:true,document:converted(104,'迟到旧文档.pdf')});await page.waitForTimeout(100);
      assert.equal(await page.getByRole('button',{name:'文档标签 迟到旧文档.pdf',exact:true}).count(),0,'stale success must not publish');
      assert.equal(await page.getByRole('button',{name:'取消图片转换',exact:true}).count(),1,'stale success must not clear current job');
      await queueCancel(page,true);await page.getByRole('button',{name:'取消图片转换',exact:true}).click();await page.getByRole('button',{name:'取消图片转换',exact:true}).waitFor({state:'hidden'});

      await queuePicker(page,{id:6,names:['first.png','second.tiff','third.jpg']});await queueConvert(page,{pending:true});
      await tool.click();await waitForRequestCount(page,'convert_image_import',6);const renders=await requestCount(page,'render_page');
      await releaseConvert(page,6,{ok:true,document:converted(106,'图片合并.pdf')});await waitForRequestCount(page,'render_page',renders+1);
      await page.getByRole('button',{name:'文档标签 图片合并.pdf',exact:true}).waitFor();
      assert.equal(await page.getByRole('button',{name:'文档标签 原有草稿.pdf',exact:true}).count(),1,'success keeps the pre-existing draft tab');
      assert.equal(await page.locator('#reader-workspace').isVisible(),true);await assertNoCard(page);
      await page.screenshot({path:`/tmp/pdfmaster-image-import-no-card-success-${width}.png`});
      await releaseConvert(page,5,{ok:false,error:'late cancelled job rejection'});await page.waitForTimeout(100);
      assert.equal(await page.getByRole('button',{name:'文档标签 图片合并.pdf',exact:true}).count(),1,'late rejection cannot remove the published current result');
      await page.close();console.log(`image no-card replay ${width}: passed (compiled WASM, synthetic native replies)`);
    }
  }finally{if(browser)await browser.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
