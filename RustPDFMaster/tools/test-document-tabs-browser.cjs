// Synthetic frontend protocol replay only; not native save/dialog or platform acceptance.
const fs=require('node:fs');
const http=require('node:http');
const path=require('node:path');
const assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root=path.resolve(__dirname,'../app/dist');
const image='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U0AAAAASUVORK5CYII=';
const server=http.createServer((req,res)=>{
  const target=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);
  if(target!==root&&!target.startsWith(root+path.sep)){res.writeHead(403);return res.end();}
  const file=target===root||target===root+path.sep?path.join(root,'index.html'):target;
  try{const content=fs.readFileSync(file);res.setHeader('Content-Type',file.endsWith('.wasm')?'application/wasm':file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(content);}catch{res.writeHead(404);res.end();}
});
(async()=>{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  let browser;
  const failures=[];
  try{
    browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_EXECUTABLE_PATH?{executablePath:process.env.PLAYWRIGHT_EXECUTABLE_PATH}:{})});
    for(const kind of ['note','ink']){
      const page=await browser.newPage({viewport:{width:1280,height:900}});
      await page.addInitScript(({kind,image})=>{
        window.requests=[];
        const document={id:7,tabs:[{id:7,name:'synthetic.pdf',dirty:true}],name:'synthetic.pdf',restored:false,info:{page_count:3,pages:Array.from({length:3},()=>({width:595,height:842,rotation:0})),dirty:true,can_undo:false,can_redo:false}};
        if(kind==='note') document.note_editor={id:9,page:2,original:{annotation_index:0,bounds:{x:10,y:10,width:24,height:24},contents:'synthetic note'}};
        else document.ink_editor={id:10,page:2,request:'synthetic-request'};
        window.__TAURI__={core:{invoke:async(command,args)=>{
          args=args instanceof Map?Object.fromEntries(args):args;
          window.requests.push({command,args});
          if(command==='document_info')return structuredClone(document);
          if(command==='render_page')return image;
          if(command==='notes_document')return {notes:[],unsupported_count:0};
          if(command==='shapes_document')return {shapes:[],unsupported_count:0};
          throw new Error('unexpected fixture command: '+command);
        }}};
      },{kind,image});
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='render_page'));
      const render=await page.evaluate(()=>window.requests.find(r=>r.command==='render_page'));
      const control=page.getByRole('button',{name:kind==='note'?'取消便笺编辑':'取消／同步手绘',exact:true});
      const visible=await control.isVisible();
      try{assert.equal(render.args.page,2,`${kind}: native editor page must override saved/default tab page`);assert.equal(visible,true,`${kind}: native cancel control must remain visible`);assert.equal(await page.getByRole('textbox',{name:'跳转页码',exact:true}).inputValue(),'3');console.log(JSON.stringify({case:kind,page:render.args.page,cancelVisible:visible,result:'pass'}));}
      catch(error){failures.push(error.message);console.log(JSON.stringify({case:kind,page:render.args.page,cancelVisible:visible,result:'fail'}));}
      await page.close();
    }
    const page=await browser.newPage({viewport:{width:1280,height:900}});
    await page.addInitScript(({image})=>{
      window.requests=[];window.nextClose='cancel';
      let active=2;
      const names=['A.pdf','synthetic-long-title-'.repeat(8)+'末尾.pdf'];
      let documents=names.map((name,index)=>({id:index+1,name,restored:false,info:{page_count:3,pages:Array.from({length:3},()=>({width:595,height:842,rotation:0})),dirty:true,can_undo:true,can_redo:false}}));
      const view=()=>{const d=documents.find(d=>d.id===active);return d?{...structuredClone(d),tabs:documents.map(d=>({id:d.id,name:d.name,dirty:d.info.dirty}))}:null;};
      window.__TAURI__={core:{invoke:async(command,args)=>{
        args=args instanceof Map?Object.fromEntries(args):args;
        window.requests.push({command,args});
        if(command==='document_info')return view();
        if(command==='activate_document'){active=args.id;return view();}
        if(command==='export_document'){documents.find(d=>d.id===active).info.dirty=false;return view();}
        if(command==='close_document'){
          if(window.nextClose==='cancel')return {closed:false,document:view()};
          documents=documents.filter(d=>d.id!==args.id);
          if(!documents.some(d=>d.id===active))active=documents.at(-1)?.id;
          return {closed:true,document:view()};
        }
        if(command==='render_page')return image;
        if(command==='notes_document')return {notes:[],unsupported_count:0};
        if(command==='shapes_document')return {shapes:[],unsupported_count:0};
        if(command==='search_document')return {query:args.query,case_sensitive:false,hits:[],pages_scanned:3,searchable_pages:3,truncated:false,limitation:''};
        throw new Error('unexpected fixture command: '+command);
      }}};
    },{image});
    const idle=()=>page.waitForFunction(()=>document.querySelector('#pdf-reader')?.getAttribute('aria-busy')==='false');
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.getByRole('button',{name:'文档标签 A.pdf',exact:true}).waitFor();await idle();
    const first=page.getByRole('button',{name:'文档标签 A.pdf',exact:true});
    const second=page.getByRole('button',{name:/文档标签 synthetic-long-title-/});
    const boxes=page.locator('.shell-document-box');
    const before=await boxes.evaluateAll(nodes=>nodes.map(n=>{const r=n.getBoundingClientRect();return{x:r.x,width:r.width};}));
    assert.equal(before[0].x,144);assert.ok(before.every(r=>r.width>=60&&r.width<=360));
    assert.equal(before[1].x,before[0].x+before[0].width);
    await page.getByRole('button',{name:'下一页',exact:true}).click();await idle();
    await page.getByRole('combobox',{name:'缩放',exact:true}).selectOption('1.5');await idle();
    await page.locator('.chrome-search').fill('second-only');
    await page.waitForFunction(()=>window.requests.some(r=>r.command==='search_document'));await idle();
    await first.click();await idle();
    assert.equal(await page.getByRole('textbox',{name:'跳转页码',exact:true}).inputValue(),'1');
    assert.equal(await page.locator('.chrome-search').inputValue(),'');
    assert.equal(await page.locator('.chrome-zoom option:checked').textContent(),'100%');
    await second.click();await idle();
    assert.equal(await page.getByRole('textbox',{name:'跳转页码',exact:true}).inputValue(),'2');
    assert.equal(await page.locator('.chrome-search').inputValue(),'second-only');
    assert.equal(await page.locator('.chrome-zoom option:checked').textContent(),'150%');
    const after=await boxes.evaluateAll(nodes=>nodes.map(n=>{const r=n.getBoundingClientRect();return{x:r.x,width:r.width};}));
    assert.deepEqual(after,before,'tab positions must not stretch or change on activation');
    await page.getByLabel('更多工具',{exact:true}).click();
    await page.getByRole('button',{name:'文档操作',exact:true}).click();
    await page.getByRole('button',{name:'导出副本',exact:true}).click();await idle();
    assert.equal((await second.textContent()).includes('•'),false,'dirty tab title must update after native reply');
    await page.getByRole('button',{name:'关闭 A.pdf',exact:true}).click();await idle();
    assert.equal(await boxes.count(),2,'cancel preserves tabs');
    await page.evaluate(()=>{window.nextClose='discard';});
    await page.getByRole('button',{name:'关闭 A.pdf',exact:true}).click();await idle();
    assert.equal(await boxes.count(),1);
    await page.locator('.shell-tab-close').click();await idle();
    assert.equal(await boxes.count(),0);
    assert.equal(await page.locator('.home-host').isVisible(),true);
    console.log(JSON.stringify({case:'multiple-tabs',pageZoomQueryRestored:true,compactLayout:true,dirtyReplyUpdated:true,cancelPreserved:true,lastCloseHome:true,result:'pass'}));
    await page.close();
    if(failures.length)throw new Error(failures.join('\n'));
  }finally{if(browser)await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error.message);process.exitCode=1;});
