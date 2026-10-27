// Compiled Rust/WASM UI with synthetic native replies, not OS picker or PDF acceptance.
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const root=path.resolve(__dirname,'../app/dist');
const server=http.createServer((req,res)=>{
  const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);
  if(file!==root&&!file.startsWith(root+path.sep)){res.writeHead(403);return res.end();}
  const target=file===root?path.join(root,'index.html'):file;
  try{res.setHeader('Content-Type',target.endsWith('.wasm')?'application/wasm':target.endsWith('.js')?'text/javascript':target.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(target));}catch{res.writeHead(404);res.end();}
});
(async()=>{
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try{
    browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_EXECUTABLE_PATH?{executablePath:process.env.PLAYWRIGHT_EXECUTABLE_PATH}:{})});
    for(const width of [1280,390]){
      const page=await browser.newPage({viewport:{width,height:900}});
      await page.addInitScript(()=>{
        window.requests=[];let next=0;
        window.__TAURI__={core:{Channel:class{constructor(cb){this.onmessage=cb;}},invoke:async(command,args)=>{
          // Mirror Tauri 2.12 process-ipc-message-fn.js, including nested Maps.
          args=JSON.parse(JSON.stringify(args,(_k,v)=>v instanceof Map?Object.fromEntries(v):v));window.requests.push({command,args});
          if(command==='document_info')return null;
          if(command==='register_file_drop_channel')return 1;
          if(command==='unregister_file_drop_channel')return null;
          if(command==='prepare_image_import'){if(window.dismissPicker){window.dismissPicker=false;return null;}return {id:++next,names:['first.png','second.tiff','third.jpg']};}
          if(command==='cancel_image_import')return true;
          if(command==='convert_image_import'){
            if(window.holdImage)return new Promise((_,reject)=>{window.releaseImage=()=>reject(new Error('synthetic cancelled conversion'));});
            if(window.applyImage)return {id:2,name:'图片合并.pdf',tabs:[{id:2,name:'图片合并.pdf',dirty:true}],restored:false,info:{page_count:3,pages:[{width:80,height:40,rotation:0},{width:40,height:80,rotation:0},{width:100,height:100,rotation:0}],dirty:true,can_undo:false,can_redo:false}};
            throw 'synthetic unsupported image: configuration retained'; // Tauri command errors reject with strings.
          }
          if(command==='render_page'||command==='render_overview_page')return 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U0AAAAASUVORK5CYII=';
          if(command==='prepare_page_overview')return 1;
          throw new Error('unexpected fixture command '+command);
        }}};
      });
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='document_info'));
      await page.getByRole('button',{name:'工具',exact:true}).click();
      const tool=page.getByRole('button',{name:'图片转 PDF',exact:true});
      assert.equal(await tool.isEnabled(),true,'independent image tool must work with no open PDF');
      await page.evaluate(()=>{window.dismissPicker=true;});await tool.click();
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='prepare_image_import'));
      await page.waitForFunction(()=>!document.querySelector('[role=dialog]'));
      await page.waitForFunction(()=>!document.querySelector('[aria-label="图片转 PDF"]').disabled);
      await tool.click();
      const dialog=page.getByRole('dialog',{name:'图片转 PDF · 本地候选功能',exact:true});await dialog.waitFor();
      const box=await dialog.boundingBox();assert.ok(box.x>=0&&box.x+box.width<=width+1);if(width===1280){assert.equal(box.width,620);assert.equal(box.height,500);}
      await dialog.getByRole('option',{name:'second.tiff',exact:true}).click();
      await dialog.getByRole('button',{name:'上移',exact:true}).click();
      assert.deepEqual(await dialog.getByRole('listbox').getByRole('option').allTextContents(),['second.tiff','first.png','third.jpg']);
      await dialog.getByRole('option',{name:'third.jpg',exact:true}).click();await dialog.getByRole('button',{name:'移除',exact:true}).click();
      await dialog.getByRole('combobox',{name:'纸型',exact:true}).selectOption('letter');
      await dialog.getByRole('combobox',{name:'方向',exact:true}).selectOption('landscape');
      await dialog.getByRole('textbox',{name:'边距（pt）',exact:true}).fill('306');
      assert.equal(await dialog.getByRole('button',{name:'创建新 PDF',exact:true}).isDisabled(),true);
      await dialog.getByRole('textbox',{name:'边距（pt）',exact:true}).fill('30');
      await dialog.getByRole('button',{name:'创建新 PDF',exact:true}).click();
      await dialog.getByText('synthetic unsupported image: configuration retained',{exact:true}).waitFor();
      const request=await page.evaluate(()=>window.requests.findLast(r=>r.command==='convert_image_import'));
      assert.deepEqual(request.args,{id:1,order:[1,0],layout:{paper:'letter',landscape:true,margin:30}});
      assert.equal(await dialog.getByRole('listbox').getByRole('option').count(),2,'failure retains configuration');
      await dialog.getByRole('combobox',{name:'纸型',exact:true}).selectOption('pixels');
      assert.equal(await dialog.getByRole('combobox',{name:'方向',exact:true}).isDisabled(),true);
      assert.equal(await dialog.getByRole('textbox',{name:'边距（pt）',exact:true}).isDisabled(),true);
      await page.evaluate(()=>{window.holdImage=true;});await dialog.getByRole('button',{name:'创建新 PDF',exact:true}).click();
      await page.waitForFunction(()=>typeof window.releaseImage==='function');
      assert.equal(await dialog.getByRole('button',{name:'移除',exact:true}).isDisabled(),true);
      await dialog.getByRole('button',{name:'取消',exact:true}).click();await dialog.waitFor({state:'hidden'});
      await tool.click();await dialog.waitFor();await page.evaluate(()=>window.releaseImage());
      assert.equal(await dialog.getByRole('listbox').getByRole('option').count(),3,'late cancelled response cannot overwrite a newer configuration');
      await page.screenshot({path:`/tmp/pdfmaster-image-import-${width}.png`});
      await dialog.press('Escape');await dialog.waitFor({state:'hidden'});
      assert.equal(await page.getByRole('button',{name:/文档标签/}).count(),0);
      await tool.click();await dialog.waitFor();
      await dialog.getByRole('combobox',{name:'纸型',exact:true}).selectOption('pixels');
      await page.evaluate(()=>{window.holdImage=false;window.applyImage=true;});
      await dialog.getByRole('button',{name:'创建新 PDF',exact:true}).click();await dialog.waitFor({state:'hidden'});
      await page.waitForFunction(()=>window.requests.some(r=>r.command==='render_page'));
      await page.getByRole('button',{name:'文档标签 图片合并.pdf',exact:true}).waitFor();
      assert.equal(await page.locator('#reader-workspace').isVisible(),true);
      const success=await page.evaluate(()=>window.requests.findLast(r=>r.command==='convert_image_import'));
      assert.deepEqual(success.args.layout,{paper:'pixels',landscape:false,margin:0});
      await page.close();console.log(`image configuration replay ${width}: passed (synthetic native replies)`);
    }
  }finally{if(browser)await browser.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
