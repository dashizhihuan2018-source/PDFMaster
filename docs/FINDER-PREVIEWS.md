# 文件未打开时显示 PDF 封面

Finder 可以直接显示 PDF 第一页的缩略图，无需先用 PDF Master 打开文件。这是 macOS 的系统 Quick Look 功能，PDF Master 保留标准 `com.adobe.pdf` 文件类型，不替换系统预览器。

列表中仍显示普通文档图标时，在这个文件夹按 ⌘J，勾选「显示图标预览」，并选择较大的列表图标。较小图标可能低于系统 PDF 预览器的最小尺寸；本机检查到最低尺寸为17点，而小列表图标为16点。较大列表图标为32点。预览是否出现还取决于文件可读性、系统缓存及 Finder 设置；加密、损坏、仅在云端的文件可能无法生成封面。

也可在源码目录运行以下辅助脚本，对指定文件夹启用系统预览与大列表图标：

```sh
osascript scripts/enable-finder-previews.applescript /absolute/path/to/folder
```

路径含空格时加引号。首次使用可能需要在系统弹窗中允许终端控制 Finder。脚本不激活 Finder、不打开文件或新窗口、不改全局默认值、不重启 Finder，也不改 PDF 内容。设置仅针对指定文件夹；可以用 ⌘J 改回小图标或关闭图标预览。macOS可能将文件夹视图设置保存到该目录的 `.DS_Store`。

后台验证：系统 Quick Look 从实际 PDF 生成了首页封面；通过 Finder 的脚本接口回读了目标文件夹的大图标及预览设置，现有窗口数量不变，PDF 的 SHA-256 不变。没有执行前台截图或目视验收，不代表所有文件夹或所有 Mac 已自动启用。

此项为系统文件夹设置与辅助脚本更新，安装版仍是 V0.1.32/build54，主程序未改动。原有版本标签、安装包和对应源码附件保持不变；辅助脚本及此说明在仓库 main。

参考：[Apple Quick Look 支持的标准文件类型](https://developer.apple.com/library/archive/documentation/UserExperience/Conceptual/Quicklook_Programming_Guide/index.html)、[缩略图最小尺寸](https://developer.apple.com/documentation/bundleresources/information-property-list/qlthumbnailminimumsize)。
