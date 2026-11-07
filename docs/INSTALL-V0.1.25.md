# PDF Master V0.1.25 / build47

Apple Silicon arm64，最低构建目标 macOS 14.0。把 PDF Master 拖入 Applications；更新前保存文档并正常退出旧版。

本版新增文件→新建窗口（⌘⌥N）、标签右键移到新窗口和标签拖出。编辑工具条与文件菜单的“删除水印”共用当前文档流程，不新增首页卡片或独立预览面板。确认后仅移除可识别的显式标准水印；不支持扫描图中烘焙水印或未标记的普通文字/图片。修改可撤销，不自动覆盖磁盘文件，请按正常保存流程保存。

保留本地账号与默认 Agnes 文本服务商。设备会话与 Key 存于本机钥匙串，不随安装包分发。更新签名后 macOS 可能要求重新授权钥匙串，请在本机核实应用来源后自行处理；勿向任何人发送系统密码。AI 发送仍需用户授权，可能产生第三方费用。

qpdf 与画布资源随包提供，无需另装 Python 或 Homebrew。复杂 PDF 保守拒绝。此包为非商业技术交流预发布，ad-hoc 签名，未获 Apple 公证；本机启动不等于其他 Mac 或浏览器首次信任验收。遇到损坏、恶意软件或组织策略阻止时停止，不关闭全局安全保护。

安装包、完整源码、哈希和验证边界：
https://github.com/dashizhihuan2018-source/PDFMaster/releases/tag/v0.1.25

具体已验与未验项目见源码 docs/MULTIWINDOW-WATERMARK-V0.1.25.md、VERIFICATION.md。版权及第三方许可沿用 COPYRIGHT.md、DISCLAIMER.md 和 App 内许可证。不分发 WPS 代码或界面资产，不宣称 WPS 全部功能等价。
