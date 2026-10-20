const {contextBridge,ipcRenderer,webUtils}=require('electron');
contextBridge.exposeInMainWorld('pdfMaster',{
  call:(action,args)=>ipcRenderer.invoke('pdfmaster',action,args),
  onAction:handler=>ipcRenderer.on('action',(_e,name)=>handler(name)),
  drop:files=>ipcRenderer.invoke('pdfmaster','drop',Array.from(files).map(f=>webUtils.getPathForFile(f)))
});
