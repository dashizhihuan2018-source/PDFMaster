// Compiled-frontend protocol replay; synthetic native replies, not PDF/OS GUI acceptance.
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const root=path.resolve(__dirname,'../app/dist');
const image='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U0AAAAASUVORK5CYII=';
const server=http.createServer((req,res)=>{
  const target=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);
  if(target!==root&&!target.startsWith(root+path.sep)){res.writeHead(403);return res.end();}
  const file=target===root?path.join(root,'index.html'):target;
  try{res.setHeader('Content-Type',file.endsWith('.wasm')?'application/wasm':file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(file));}catch{res.writeHead(404);res.end();}
});
(async()=>{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
  try{
    browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_EXECUTABLE_PATH?{executablePath:process.env.PLAYWRIGHT_EXECUTABLE_PATH}:{})});
    for(const width of [1280,390]){
      const page=await browser.newPage({viewport:{width,height:900}});
      await page.addInitScript(({image})=>{
        window.requests=[];let next=0,current=0;
        const d={id:1,name:'synthetic-overview.pdf',tabs:[{id:1,name:'synthetic-overview.pdf',dirty:false}],restored:false,info:{page_count:40,pages:Array.from({length:40},(_,i)=>({width:595,height:i%3===0?595:842,rotation:i%4===0?90:0})),dirty:false,can_undo:false,can_redo:false}};
        window.__TAURI__={core:{Channel:class{constructor(callback){this.onmessage=callback;}},invoke:async(command,args)=>{
          args=args instanceof Map?Object.fromEntries(args):args;window.requests.push({command,args});
          if(command==='register_file_drop_channel'){window.nativeDrop=p=>args.channel.onmessage(p);return 1;}
          if(command==='unregister_file_drop_channel'){delete window.nativeDrop;return null;}
          if(command==='document_info')return structuredClone(d);
          if(command==='render_page'){if(window.failRender){window.failRender=false;throw new Error('synthetic preview failure');}return image;}
          if(command==='copy_pages_document'){
            assertToken(args.token);if(args.document!==d.id)throw new Error('foreign document');
            window.pageClipboard=args.pages.slice().sort((a,b)=>a-b).map(i=>structuredClone(d.info.pages[i]));
            if(!args.cut)return {pages:args.pages.length,document:null};
            d.info.pages=d.info.pages.filter((_,i)=>!args.pages.includes(i));d.info.page_count=d.info.pages.length;d.warning=null;current=0;
            return {pages:args.pages.length,document:structuredClone(d)};
          }
          if(command==='paste_pages_document'){
            assertToken(args.token);if(!window.pageClipboard)throw new Error('synthetic external replacement');
            d.info.pages.splice(args.index,0,...structuredClone(window.pageClipboard));d.info.page_count=d.info.pages.length;current=0;return structuredClone(d);
          }
          if(command==='insert_pdf_document'||command==='insert_pdfs_document')return null;
          if(command==='cancel_file_drop')return true;
          if(command==='import_file_drop'){
            if(window.holdDrop){window.holdDrop=false;return new Promise((_,reject)=>{window.releaseDrop=()=>reject(new Error('synthetic cancelled import'));});}
            if(window.failDrop){window.failDrop=false;throw new Error('synthetic invalid batch');}
            d.info.pages.splice(args.index,0,{width:111,height:222,rotation:0},{width:333,height:444,rotation:0});
            d.info.page_count=d.info.pages.length;d.info.dirty=true;d.warning=null;current=0;return structuredClone(d);
          }
          if(command==='notes_document')return {notes:[],unsupported_count:0};
          if(command==='shapes_document')return {shapes:[],unsupported_count:0};
          if(command==='prepare_page_overview'){current=++next;return current;}
          if(command==='render_overview_page'){
            assertToken(args.token);
            if(window.holdPreview){window.holdPreview=false;return new Promise(resolve=>{window.releasePreview=()=>resolve(image+'#stale-preview');});}
            return image;
          }
          if(command==='move_overview_pages'){
            assertToken(args.token);const selected=[...args.pages].sort((a,b)=>a-b),old=d.info.pages;
            const rest=old.filter((_,i)=>!selected.includes(i));const start=args.insertion-selected.filter(i=>i<args.insertion).length;
            rest.splice(start,0,...selected.map(i=>old[i]));d.info.pages=rest;d.info.dirty=true;d.info.can_undo=true;d.tabs[0].dirty=true;current=0;return structuredClone(d);
          }
          if(command==='batch_pages'){
            if(!window.applyBatch)return {kind:'cancelled'};
            d.info.dirty=true;d.info.can_undo=true;d.warning='synthetic checkpoint warning';current=0;
            return {kind:'applied',document:structuredClone(d),affected_count:2};
          }
          throw new Error('unexpected fixture command: '+command);
        }}};
        function assertToken(value){if(value!==current||!value)throw new Error('stale synthetic snapshot');}
      },{image});
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='render_page'));
      const toggle=page.getByRole('button',{name:'页面预览',exact:true});
      assert.equal(await toggle.isEnabled(),true,'the original overview entry must be real, not disabled');
      await toggle.click();const grid=page.locator('#pdf-overview');await grid.waitFor({state:'visible'});
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='render_overview_page'));
      assert.equal(await page.locator('#pdf-reader').isVisible(),false);
      assert.equal(await page.locator('#pdf-reader-sidebar').isVisible(),false);
      assert.equal(await page.getByRole('combobox',{name:'缩放',exact:true}).isDisabled(),true);
      const selected=()=>page.locator('.overview-tile[aria-selected="true"]').evaluateAll(nodes=>nodes.map(n=>Number(n.dataset.page)));
      const tile=i=>page.locator(`.overview-tile[data-page="${i}"]`);
      await tile(1).click();await tile(3).click({modifiers:['Control']});assert.deepEqual(await selected(),[1,3]);
      await tile(5).click({modifiers:['Shift']});assert.deepEqual(await selected(),[3,4,5]);
      await tile(1).click({modifiers:['Control','Shift']});assert.deepEqual(await selected(),[1,2,3,4,5]);
      await tile(1).click();await tile(3).click({modifiers:['Control']});
      const a=await tile(3).boundingBox(),b=await tile(0).boundingBox();
      await page.mouse.move(a.x+a.width/2,a.y+20);await page.mouse.down();await page.mouse.move(b.x+4,b.y+20,{steps:8});await page.mouse.up();
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='move_overview_pages'));
      const moved=await page.evaluate(()=>window.requests.find(r=>r.command==='move_overview_pages'));
      assert.deepEqual(moved.args.pages,[1,3]);assert.equal(moved.args.insertion,0);
      await page.waitForFunction(()=>document.querySelectorAll('.overview-tile[aria-selected="true"]').length===2);
      assert.deepEqual(await selected(),[0,1]);
      await grid.focus();await page.keyboard.press('Delete');await page.waitForFunction(()=>window.requests.some(r=>r.command==='batch_pages'));
      assert.deepEqual(await selected(),[0,1],'cancelled native delete retains selection');
      await tile(1).click();await tile(3).click({modifiers:['Control']});
      await page.evaluate(()=>{window.applyBatch=true;});
      await page.getByRole('button',{name:'工具 · 旋转',exact:true}).click();
      await page.waitForFunction(()=>window.requests.filter(r=>r.command==='batch_pages').length===2);
      await page.waitForTimeout(100);
      assert.deepEqual(await selected(),[1,3],'rotation retains the original selection');
      assert.ok(await page.getByText('synthetic checkpoint warning',{exact:true}).count(),'checkpoint warning is not replaced by generic success');
      const copyButton=page.getByRole('button',{name:'工具 · 复制',exact:true}),pasteButton=page.getByRole('button',{name:'工具 · 粘贴',exact:true});
      assert.equal(await copyButton.isEnabled(),true,'page copy must invoke the system clipboard, not stay disabled');
      await copyButton.click();await page.waitForFunction(()=>window.requests.some(r=>r.command==='copy_pages_document'));
      assert.deepEqual(await page.evaluate(()=>window.requests.find(r=>r.command==='copy_pages_document').args.pages),[1,3]);
      await page.evaluate(()=>{window.pageClipboard=null;});await pasteButton.click();
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='paste_pages_document'));
      assert.deepEqual(await selected(),[1,3],'external clipboard replacement cannot paste a stale private cache');
      await copyButton.click();await pasteButton.click();
      await page.waitForFunction(()=>document.querySelector('.overview-tile[data-page="4"][aria-selected="true"]'));
      assert.deepEqual(await selected(),[4,5]);
      assert.equal(await page.evaluate(()=>window.requests.filter(r=>r.command==='paste_pages_document').at(-1).args.index),4);
      await tile(1).click();await tile(3).click({modifiers:['Control']});
      const moveCount=await page.evaluate(()=>window.requests.filter(r=>r.command==='move_overview_pages').length);
      await grid.evaluate(el=>el.addEventListener('pointerdown',event=>{window.cancelPointer=event.pointerId;},{once:true}));
      const cancelTile=await tile(1).boundingBox();
      await page.mouse.move(cancelTile.x+20,cancelTile.y+20);await page.mouse.down();
      await page.mouse.move(cancelTile.x+40,cancelTile.y+30,{steps:3});
      await grid.evaluate(el=>el.dispatchEvent(new PointerEvent('pointercancel',{pointerId:window.cancelPointer,pointerType:'mouse',bubbles:true})));
      await page.mouse.up();
      assert.equal(await page.evaluate(()=>window.requests.filter(r=>r.command==='move_overview_pages').length),moveCount,'cancelled pointer must not commit a move');
      await page.evaluate(()=>{window.holdPreview=true;});
      await page.getByRole('button',{name:'放大',exact:true}).click();
      await page.waitForFunction(()=>typeof window.releasePreview==='function');
      await page.getByRole('button',{name:'放大',exact:true}).click();
      const heldCount=await page.evaluate(()=>window.requests.filter(r=>r.command==='render_overview_page').length);
      await page.evaluate(()=>window.releasePreview());
      await page.waitForFunction(count=>window.requests.filter(r=>r.command==='render_overview_page').length>count,heldCount);
      assert.equal(await page.locator('.overview-tile img[src$="#stale-preview"]').count(),0,'late preview from the previous zoom cannot fill the new generation');
      await page.getByRole('button',{name:'适合页面',exact:true}).click();
      await tile(2).dblclick();await page.locator('#pdf-reader').waitFor({state:'visible'});
      assert.equal(await page.getByRole('textbox',{name:'跳转页码',exact:true}).inputValue(),'3');
      await toggle.click();await grid.waitFor({state:'visible'});
      await tile(0).click();await grid.focus();
      const columns=await grid.evaluate(el=>Math.max(1,Math.round((Math.max(1,el.clientWidth-40)+16)/196)));
      for(let i=0;i<5;i++)await page.keyboard.press('Shift+ArrowDown');
      const lead=Math.min(39,columns*5);
      assert.equal(await tile(lead).isVisible(),true,'keyboard focus endpoint must be in the virtual viewport');
      const focused=await tile(lead).boundingBox(),viewport=await grid.boundingBox();
      assert.ok(focused.y+focused.height>viewport.y&&focused.y<viewport.y+viewport.height,'keyboard range must scroll to its focus endpoint');
      await page.keyboard.press('Enter');await page.locator('#pdf-reader').waitFor({state:'visible'});
      assert.equal(await page.getByRole('textbox',{name:'跳转页码',exact:true}).inputValue(),String(lead+1));
      await toggle.click();await grid.waitFor({state:'visible'});
      await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
      await grid.evaluate(el=>{el.scrollTop=el.scrollHeight;el.dispatchEvent(new Event('scroll'));});
      await tile(39).waitFor({state:'visible'});assert.ok(await page.locator('.overview-tile').count()<40);
      await grid.evaluate(el=>{el.scrollTop=0;el.dispatchEvent(new Event('scroll'));});
      await tile(1).click();await tile(3).click({modifiers:['Control']});
      await page.getByRole('button',{name:'工具 · 摘录',exact:true}).click();
      assert.equal(await page.getByRole('textbox',{name:'批量页码范围',exact:true}).inputValue(),'2,4');
      await page.getByRole('button',{name:'在当前页后插入 PDF',exact:true}).click();
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='insert_pdf_document'));
      assert.equal(await page.evaluate(()=>window.requests.find(r=>r.command==='insert_pdf_document').args.index),4,'panel insertion uses the last overview selection, not the hidden reader');
      assert.deepEqual(await selected(),[1,3],'cancelled insertion retains selection');
      await page.evaluate(()=>{window.failRender=true;});await grid.focus();await page.keyboard.press('Control+x');
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='copy_pages_document'&&r.args.cut));
      await page.waitForFunction(()=>document.querySelector('#pdf-overview').getAttribute('aria-busy')==='false');
      const cutStatus=await page.locator('.statusbar [role="status"]').innerText();
      assert.ok(cutStatus.includes('synthetic preview failure'),'committed cut must not conceal the failed preview');
      assert.ok(cutStatus.includes('已剪切')&&cutStatus.includes('可撤销'),'preview failure must not imply that the completed cut was rolled back');
      assert.equal(await page.evaluate(()=>typeof window.nativeDrop),'function','native file-drop listener must be registered');
      await grid.evaluate(el=>{el.scrollTop=0;el.dispatchEvent(new Event('scroll'));});await tile(0).waitFor({state:'visible'});
      const dropBox=await tile(0).boundingBox(),dropPoint={x:dropBox.x+dropBox.width-4,y:dropBox.y+20};
      await page.evaluate(p=>window.nativeDrop({kind:'over',id:null,...p,error:null}),dropPoint);
      assert.equal(await page.locator('.overview-insertion').count(),1,'native hover exposes the original insertion edge');
      await page.evaluate(p=>{window.failDrop=true;window.nativeDrop({kind:'drop',id:101,...p,error:null});},dropPoint);
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='import_file_drop'));
      assert.equal(await page.evaluate(()=>window.requests.find(r=>r.command==='import_file_drop').args.index),1,'right half inserts after the hit page');
      await page.waitForFunction(()=>document.querySelector('#pdf-overview').getAttribute('aria-busy')==='false');
      assert.ok((await page.locator('.statusbar [role="status"]').innerText()).includes('synthetic invalid batch'));
      await page.evaluate(p=>{window.holdDrop=true;window.nativeDrop({kind:'drop',id:102,...p,error:null});},dropPoint);
      const cancelImport=page.getByRole('button',{name:'取消文件导入',exact:true});await cancelImport.waitFor({state:'visible'});await cancelImport.click();
      await page.waitForFunction(()=>document.querySelector('#pdf-overview').getAttribute('aria-busy')==='false');
      assert.ok(await page.evaluate(()=>window.requests.some(r=>r.command==='cancel_file_drop'&&r.args.id===102)));
      await page.evaluate(p=>window.nativeDrop({kind:'drop',id:103,...p,error:null}),dropPoint);
      await page.waitForFunction(()=>document.querySelector('.overview-tile[data-page="1"][aria-selected="true"]'));
      assert.deepEqual(await selected(),[1,2],'accepted drop selects its inserted range');
      await page.waitForFunction(()=>document.querySelector('#pdf-overview').getAttribute('aria-busy')==='false');
      const acceptedStatus=await page.locator('.statusbar [role="status"]').innerText();
      await page.evaluate(()=>{window.releaseDrop();delete window.releaseDrop;});await page.waitForTimeout(30);
      assert.equal(await page.locator('.statusbar [role="status"]').innerText(),acceptedStatus,'cancelled old read cannot replace the newer accepted import');
      await toggle.click();await page.locator('#pdf-reader').waitFor({state:'visible'});
      const reader=await page.locator('#pdf-reader').boundingBox();
      await page.evaluate(p=>window.nativeDrop({kind:'drop',id:104,...p,error:null}),{x:reader.x+20,y:reader.y+20});
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='import_file_drop'&&r.args.id===104));
      assert.equal(await page.evaluate(()=>window.requests.find(r=>r.command==='import_file_drop'&&r.args.id===104).args.index),2,'reader drop follows its current page');
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'no body horizontal overflow');
      console.log(JSON.stringify({case:'overview',width,selection:true,snapshotMove:true,deleteCancel:true,openPage:true,virtualScroll:true,result:'pass'}));
      await page.close();
    }
  }finally{if(browser)await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
