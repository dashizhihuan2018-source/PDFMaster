# PDF Master

专注阅读、自由批注、有序组织。

PDF Master 是面向 macOS 的独立 PDF 工作台，将阅读、批注、页面整理与本地画布连接在同一工作流中。文档在本机管理；需要网络的可选服务具有单独的数据传输与授权边界。

本仓库提供 build 19 R5 源码与构建工具，持续更新，不分发安装包或用户文档。系统与功能兼容范围见 [兼容性与验证说明](VERIFICATION.md)，发布目的、隐私及第三方权利说明见 [使用与权利声明](DISCLAIMER.md)。本项目与 Apple、Readdle 等第三方不存在合作或官方背书关系。

## 功能与限制

支持基础阅读、搜索/大纲/缩略图、批注与手绘/签名、基本页面操作、图片与文字类文件导入、PNG/JPEG 导出，以及自包含 `.pdfmasterboard` 本地画布。复杂 PDF、表单、加密/签名与主动内容有保守拒绝边界。内容编辑、OCR 与格式转换只覆盖明确子集，不承诺任意 Office 原版式转换、完整 OCR 互操作或全功能兼容。请使用自有测试副本并存储副本后独立核对。

原生本地画布与首页嵌入的在线画布网站是不同系统；在线网站需要网络，其登录、服务和数据政策独立，不是本应用离线能力或开源资源。

## 编译与打包

需要 macOS 14+ 和支持 Swift tools 6 的 Xcode/Command Line Tools。内部 Swift target 名保留 PaperDesk/PaperCore。

```sh
swift build
swift test
bash scripts/build-app.sh
```

打包结果为 `dist/PDF Master.app`。脚本只编译并创建新 bundle，不安装、不运行、不清理旧版本；已有目标会拒绝覆盖。使用 macOS 本地 ad-hoc 签名，不改变系统安全策略。

qpdf 不捆绑进此源码目录或默认 App bundle。页码、无损优化等依赖外部 qpdf，应用通过 `PDFMASTER_QPDF` 的绝对可执行路径或当前 PATH 发现它。打包时可传 `--qpdf /absolute/path/to/qpdf` 仅检查该可选依赖，不复制二进制或依赖库；图形启动环境 PATH 可能不同，可直接运行 `dist/PDF Master.app/Contents/MacOS/PaperDesk` 并显式设置环境。没有 qpdf 时相关操作明确拒绝，部分测试明确 skip。

```sh
PDFMASTER_QPDF=/absolute/path/to/qpdf swift test
bash scripts/build-app.sh --qpdf /absolute/path/to/qpdf
```

### 可选内容编辑运行时

内容编辑研究子集使用包外 Python 3.12 与 PyMuPDF 1.27.2.3，当前适配器要求标准 framework Python 和该用户的 Library/Python 安装位置；不是自包含便携功能。运行时不在此仓库，也不自动下载。PyMuPDF 为 AGPL-3.0 或商业授权；调用外部进程不构成许可豁免。请根据最终使用/分发模式自行确认合规，不能把本目录公开视为商业许可证批准。[PyMuPDF 官方许可](https://pymupdf.io/licensing)。

内容编辑集成测试只有显式设置 `PDFMASTER_CONTENT_FIXTURE`（自有测试 PDF）、`PDFMASTER_CONTENT_REPLACEMENT`（自有 PNG）且运行时可用才执行；否则按原因 skip。夹具须有测试断言要求的文本/图片结构，不能用私人业务 PDF。其他图像导入测试现场生成合成图片，图标测试使用 Resources 内的正式图标，不依赖开发工作区。

## 网络、账户和隐私

AI 功能使用用户自行提供的 DMX Key；Key 置于本机非同步 Keychain，仓库不含可用 API Key。生成需明确勾选同意，可能计费。图片/视频发送输入提示词；PDF Copilot 发送选定范围的可复制文字（默认当前页、有长度上限），不是 PDF 二进制或页面图片。回答未经核实；停止本机请求不能撤回已发送内容，也不保证停止远端计费。

应用保留一个有意公开的默认本机账户/口令。**它不是秘密，也不是安全隔离；首次登录后立即修改。** 本机账户不是操作系统账户、云身份或 AI 长期记忆，同一 macOS 用户下的进程不因此隔离。公开源码将公开这个默认值；不要把真实凭据、用户偏好、最近文档或 Keychain 导出提交到仓库。

## 源码与素材许可状态

此源码目前仅公开供查看；权利人尚未指定项目开源许可证，因此不应推定获得使用、修改、复制或再分发授权。此声明不改变第三方组件各自已授予的许可，也不声称拥有第三方或生成内容的排他权利。请先取得权利人明确授权再作其他用途。

SwiftyDraw 局部平滑路径适配保留 MIT 许可证与固定来源说明，见 `Resources/Licenses/SwiftyDraw/`。品牌与 56 张工具图标的来源说明见 `Resources/ASSETS.md`。系统框架/字体不随仓库复制。qpdf、libjpeg-turbo 和 PyMuPDF 未捆绑在此源码；若另行分发它们或其二进制，请遵守各自许可证并保留完整 notice，不能拿 SwiftyDraw MIT 作为整个产品许可。

© Nankong

## 持续更新

源码、交互体验与使用文档持续维护，以当前默认分支及实际验证范围为准。反馈 PDF 兼容性问题时，请使用人工生成的最小示例，勿上传私人文档、签名、截图中的个人资料或账号信息。
