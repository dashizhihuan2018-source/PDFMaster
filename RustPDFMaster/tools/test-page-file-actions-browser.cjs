// Compiled WASM/UI protocol replay with synthetic IPC; NOT Windows native GUI QA.
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const root=path.resolve(__dirname,'../app/dist');
const image='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U0AAAAASUVORK5CYII=';
const server=http.createServer((req,res)=>{const target=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);if(target!==root&&!target.startsWith(root+path.sep)){res.writeHead(403);return res.end();}const file=target===root?path.join(root,'index.html'):target;try{res.setHeader('Content-Type',file.endsWith('.wasm')?'application/wasm':file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(file));}catch{res.writeHead(404);res.end();}});
(async()=>{await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;try{
  browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_EXECUTABLE_PATH?{executablePath:process.env.PLAYWRIGHT_EXECUTABLE_PATH}:{})});
  for(const width of [1280,390]){
    const page=await browser.newPage({viewport:{width,height:900}});
    await page.addInitScript(({image})=>{window.requests=[];const d={id:1,name:'synthetic-page-actions.pdf',tabs:[{id:1,name:'synthetic-page-actions.pdf',dirty:false}],restored:false,info:{page_count:3,pages:Array.from({length:3},()=>({width:595,height:842,rotation:0})),dirty:false,can_undo:false,can_redo:false}};
      window.__TAURI__={core:{Channel:class{constructor(c){this.onmessage=c;}},invoke:async(command,args)=>{args=args instanceof Map?Object.fromEntries(args):args;window.requests.push({command,args});
        if(command==='document_info')return structuredClone(d);
        if(command==='render_page'||command==='render_overview_page')return image;
        if(command==='register_file_drop_channel'||command==='prepare_page_overview')return 1;
        if(command==='unregister_file_drop_channel')return null;
        if(command==='notes_document')return {notes:[],unsupported_count:0};
        if(command==='shapes_document')return {shapes:[],unsupported_count:0};
        if(command==='prompt_page_action'){if(window.moveSuccess&&args.operation==='move')return {document:structuredClone(d),target_page:2};return {document:null,target_page:null};}
        if(['extract_pages_document','insert_pdf_document','insert_pdfs_document','export_document','new_document','edit_document','batch_pages'].includes(command))return command==='batch_pages'?{kind:'cancelled'}:null;
        throw new Error('unexpected fixture command '+command);
      }}};
    },{image});
    await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>window.requests.some(r=>r.command==='render_page'));
    assert.equal(await page.locator('.inspector,.migration-note,#tool-batch').count(),0,'parameter inspector must be physically absent');
    const menu=async(name)=>{await page.getByLabel('更多工具',{exact:true}).click();await page.getByRole('button',{name,exact:true}).click();};
    for(const [name,operation] of [['提取页面…','extract'],['旋转页面范围…','rotate_range'],['删除页面范围…','delete_range'],['移动当前页…','move']]){
      await menu('页面操作');await page.getByRole('button',{name,exact:true}).click();await page.waitForFunction(op=>window.requests.some(r=>r.command==='prompt_page_action'&&r.args.operation===op),operation);
      assert.equal(await page.locator('.workbench-toolbar details[open]').count(),0,'action closes menus');
    }
    assert.equal(await page.getByRole('textbox',{name:'跳转页码',exact:true}).inputValue(),'1','cancel leaves page unchanged');
    await page.evaluate(()=>window.moveSuccess=true);await menu('页面操作');await page.getByRole('button',{name:'移动当前页…',exact:true}).click();await page.waitForFunction(()=>document.querySelector('[aria-label="跳转页码"]').value==='3');
    const last=await page.evaluate(()=>window.requests.filter(r=>r.command==='prompt_page_action').at(-1));assert.equal(last.args.page,0);
    await page.getByRole('button',{name:'页面预览',exact:true}).click();await page.locator('#pdf-overview').waitFor({state:'visible'});
    await page.locator('.overview-tile[data-page="0"]').click();await page.locator('.overview-tile[data-page="2"]').click({modifiers:['Control']});
    const count=await page.evaluate(()=>window.requests.filter(r=>r.command==='prompt_page_action').length);
    await page.getByRole('button',{name:'工具 · 摘录',exact:true}).click();await page.waitForFunction(()=>window.requests.some(r=>r.command==='extract_pages_document'));
    assert.equal(await page.evaluate(()=>window.requests.find(r=>r.command==='extract_pages_document').args.range),'1,3');assert.equal(await page.evaluate(()=>window.requests.filter(r=>r.command==='prompt_page_action').length),count,'overview selected extraction skips range prompt');
    await menu('文档操作');await page.getByRole('button',{name:'新建',exact:true}).click();await page.waitForFunction(()=>window.requests.some(r=>r.command==='new_document'));
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'no whole viewport horizontal overflow');
    await page.close();console.log(`PASS page/File no-card compiled UI ${width}`);
  }
}finally{if(browser)await browser.close();await new Promise(r=>server.close(r));}})().catch(e=>{console.error(e);process.exitCode=1;});
