'use strict';
const {app,BrowserWindow,Menu,dialog,ipcMain,shell}=require('electron');
const fs=require('node:fs/promises'),path=require('node:path'),crypto=require('node:crypto');
const core=require('./core.cjs'),qpdf=require('./qpdf.cjs');
const entry=path.join(__dirname,'renderer/index.html');let win,bytes=null,title='未命名.pdf',history=[],future=[],dirty=false;
const smoke=process.argv.includes('--smoke-test');
let serial=Promise.resolve();const resourceRoot=()=>app.isPackaged?process.resourcesPath:path.join(__dirname,'runtime');
if(smoke)app.setPath('userData',path.join(require('node:os').tmpdir(),'PDFMaster-smoke-'+process.pid));
function data(){return {bytes:bytes?Array.from(bytes):null,title,dirty,undo:!!history.length,redo:!!future.length};}
function set(next,name,remember=true){if(remember&&bytes){history.push(bytes);while(history.reduce((n,b)=>n+b.length,0)>256*1024*1024)history.shift();history=history.slice(-20);}else if(!remember)history=[];future=[];bytes=next;title=name||title;dirty=remember;return data();}
async function discard(){return !dirty || (await dialog.showMessageBox(win,{type:'warning',message:'放弃当前未保存的修改？',buttons:['保留编辑','放弃修改'],defaultId:0,cancelId:0})).response===1;}
async function read(file){const s=await fs.stat(file);if(!s.isFile()||s.size>core.MAX||path.extname(file).toLowerCase()!=='.pdf')throw Error('请选择 256 MB 以内的 PDF');return new Uint8Array(await fs.readFile(file));}
async function pickPDF(){const p=await dialog.showOpenDialog(win,{filters:[{name:'PDF',extensions:['pdf']}],properties:['openFile']});if(p.canceled||!await discard())return null;return set(await read(p.filePaths[0]),path.basename(p.filePaths[0]),false);}
async function atomicSave(file,buffer){const tmp=path.join(path.dirname(file),'.pdfmaster-'+crypto.randomUUID()+'.tmp');try{await fs.writeFile(tmp,buffer,{flag:'wx',mode:0o600});await fs.rename(tmp,file);}finally{await fs.unlink(tmp).catch(()=>{});}}
async function handle(action,args={}){
  if(action==='open')return pickPDF();
  if(action==='drop'){if(!Array.isArray(args)||args.length!==1||!await discard())return null;return set(await read(args[0]),path.basename(args[0]),false);}
  if(action==='state')return data();
  if(action==='compress'){if(!bytes)throw Error('请先打开PDF');await core.load(bytes);return set(new Uint8Array(await qpdf.transform(bytes,'compress',{},resourceRoot())));}
  if(action==='encrypt'){if(!bytes)throw Error('请先打开PDF');await core.load(bytes);qpdf.password(args.password);const p=await dialog.showSaveDialog(win,{defaultPath:title.replace(/\.pdf$/i,'')+'-protected.pdf',filters:[{name:'PDF',extensions:['pdf']}]});if(p.canceled)return false;const encrypted=await qpdf.transform(bytes,'encrypt',{password:args.password},resourceRoot());await atomicSave(p.filePath,encrypted);return {saved:true};}
  if(action==='new'){if(!await discard())return null;return set(await core.blank(),'未命名.pdf',false);}
  if(action==='save'){if(!bytes)throw Error('请先打开 PDF');const p=await dialog.showSaveDialog(win,{defaultPath:title,filters:[{name:'PDF',extensions:['pdf']}]});if(p.canceled)return false;await atomicSave(p.filePath,Buffer.from(bytes));title=path.basename(p.filePath);dirty=false;return data();}
  if(action==='modify'){if(!bytes)throw Error('请先打开 PDF');if(!args||typeof args.op!=='string')throw Error('无效操作');return set(await core.modify(bytes,args.op,args.args));}
  if(action==='undo'){if(history.length){future.push(bytes);bytes=history.pop();dirty=true;}return data();}
  if(action==='redo'){if(future.length){history.push(bytes);bytes=future.pop();dirty=true;}return data();}
  if(action==='merge'){const p=await dialog.showOpenDialog(win,{filters:[{name:'PDF',extensions:['pdf']}],properties:['openFile','multiSelections']});if(p.canceled||p.filePaths.length<2||!await discard())return null;return set(await core.merge(await Promise.all(p.filePaths.map(read))),'合并文档.pdf',false);}
  if(action==='images'){const p=await dialog.showOpenDialog(win,{filters:[{name:'PNG / JPEG',extensions:['png','jpg','jpeg']}],properties:['openFile','multiSelections']});if(p.canceled||!await discard())return null;const inputs=[];for(const f of p.filePaths){const s=await fs.stat(f);if(s.size>50*1024*1024)throw Error('图片过大');inputs.push(await fs.readFile(f));}return set(await core.images(inputs),'图片转换.pdf',false);}
  if(action==='exportImage'){if(typeof args.png!=='string'||args.png.length>128*1024*1024||!/^data:image\/png;base64,[A-Za-z0-9+/=]+$/.test(args.png))throw Error('无效图片');const p=await dialog.showSaveDialog(win,{defaultPath:'PDFMaster-page.png',filters:[{name:'PNG',extensions:['png']}]});if(!p.canceled)await atomicSave(p.filePath,Buffer.from(args.png.split(',')[1],'base64'));return !p.canceled;}
  if(action==='onlineCanvas'){const remote=new BrowserWindow({width:1250,height:850,webPreferences:{sandbox:true,contextIsolation:true,nodeIntegration:false}});remote.webContents.setWindowOpenHandler(({url})=>{if(/^https:\/\//.test(url))shell.openExternal(url);return {action:'deny'};});remote.loadURL('https://infinitecanvas.maip.top/canvas?mode=recent');return true;}
  if(action==='brand')return {version:'V0.1.5-win-preview1',status:'Windows 适配预览；不等同 Mac 功能对等验收',limitations:['原文内容编辑','OCR','本机账号/AI','无边记文件格式互通','完整签名/批注互操作']};
  throw Error('该功能尚未完成 Windows 适配，未执行任何修改');
}
app.whenReady().then(async()=>{
  win=new BrowserWindow({width:1280,height:860,minWidth:900,minHeight:620,title:'PDF Master',icon:app.isPackaged?path.join(process.resourcesPath,'brand.png'):path.join(__dirname,'../Resources/Branding/PDFMaster.png'),show:!smoke,webPreferences:{preload:path.join(__dirname,'preload.cjs'),sandbox:true,contextIsolation:true,nodeIntegration:false}});
  const trusted=require('node:url').pathToFileURL(entry).href;
  ipcMain.handle('pdfmaster',(event,action,args)=>{if(event.senderFrame!==win.webContents.mainFrame||event.senderFrame.url!==trusted)throw Error('Untrusted frame');const result=serial.then(()=>handle(action,args));serial=result.catch(()=>{});return result;});
  win.webContents.setWindowOpenHandler(()=>({action:'deny'}));
  win.webContents.on('will-navigate',(e,url)=>{if(url!==trusted)e.preventDefault();});
  let quitting=false;win.on('close',e=>{if(dirty&&!quitting){e.preventDefault();discard().then(ok=>{if(ok){quitting=true;win.close();}});}});
  const act=name=>()=>win.webContents.send('action',name);
  Menu.setApplicationMenu(Menu.buildFromTemplate([{label:'文件',submenu:[{label:'打开…',accelerator:'CmdOrCtrl+O',click:act('open')},{label:'新建',accelerator:'CmdOrCtrl+N',click:act('new')},{label:'保存/另存为…',accelerator:'CmdOrCtrl+S',click:act('save')},{type:'separator'},{role:'quit'}]},{label:'编辑',submenu:[{label:'撤销',accelerator:'CmdOrCtrl+Z',click:act('undo')},{label:'重做',accelerator:'CmdOrCtrl+Shift+Z',click:act('redo')},{type:'separator'},{role:'copy'},{role:'paste'},{role:'selectAll'}]},{label:'视图',submenu:[{role:'togglefullscreen'},{role:'zoomIn'},{role:'zoomOut'},{role:'resetZoom'}]}]));
  if(smoke){bytes=await core.blank();bytes=await core.modify(bytes,'annotate',{page:0,kind:'Square',rect:[50,50,180,140]});title='synthetic-smoke.pdf';const compressed=await qpdf.transform(bytes,'compress',{},resourceRoot());if((await core.load(compressed)).getPageCount()!==1)throw Error('Compression round trip failed');const protectedPDF=await qpdf.transform(bytes,'encrypt',{password:'synthetic-ci-password'},resourceRoot());const decoded=await qpdf.transform(protectedPDF,'decrypt',{password:'synthetic-ci-password'},resourceRoot());if((await core.load(decoded)).getPageCount()!==1)throw Error('Encryption round trip failed');}
  await win.loadFile(entry);
  if(smoke){try{const result=await win.webContents.executeJavaScript('window.runSmoke()');if(!result.rendered||result.annotations<1)throw Error('PDF render/annotation smoke failed');const folder=process.env.SMOKE_OUTPUT||app.getPath('temp');await fs.mkdir(folder,{recursive:true});await fs.writeFile(path.join(folder,'pdfmaster-smoke.json'),JSON.stringify({status:'automated-smoke-only',result,platform:process.platform,arch:process.arch,version:app.getVersion()},null,2));await fs.writeFile(path.join(folder,'pdfmaster-smoke.png'),(await win.webContents.capturePage()).toPNG());app.exit(0);}catch(e){console.error(e);app.exit(1);}}
}).catch(e=>{console.error(e);app.exit(1);});
app.on('window-all-closed',()=>app.quit());
