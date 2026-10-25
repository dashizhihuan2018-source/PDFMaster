# 内置无限画布前端

PDF Master V0.1.19/build41 的公开版本地 WebKit UI，固定源 `pdfmaster-canvas://workspace`。沿用 V0.1.7 已审查公开前端；`npm ci --ignore-scripts` 后运行 `npm run typecheck && npm run build`，生成 `../Resources/CanvasWeb/`；依赖和生成文件不入库。

基于 InfiniteCanvas（basketikun，MIT）改编，许可证见 [LICENSE](LICENSE)。公开树不附画布上游项目演示图片/视频或第三方提示词 JSON 快照；用户可自行导入有权使用的素材。未把网站服务端 Agent、云同步、远程插件安装或外部渠道作为桌面能力提供。

用户只在原生 Key 设置中输入自己的 DMX Key；JS 不读取真实 Key。详细说明见 [使用教程](../docs/无限画布使用教程.md)。
