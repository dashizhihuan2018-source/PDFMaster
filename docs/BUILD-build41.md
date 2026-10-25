# V0.1.19/build41 完整公开源码构建

本树为 macOS build41 完整项目源码，不是只提供旧源码的下载页。`Sources/` 包括全部原生产品代码；`CanvasWeb/` 是公开安装包使用的完整已审查前端；`Resources/ContentEdit/` 包括内容编辑和字体兼容脚本。测试、正式图标和第三方许可证随树提供。iOS、Windows 历史预览不属于本次 macOS 安装包，不将它们冒称同步升级。

## 环境和顺序

需要 macOS 14+、Swift tools 6、Node.js 22.12+/npm。内容编辑测试另需 framework Python 3.12、用户级 PyMuPDF 1.27.2.3 与 fontTools（本机验证 4.63.0），运行时及其许可边界见 README。无真实服务 Key、无付费 API 请求。

```sh
(cd CanvasWeb && npm ci --ignore-scripts && npm run typecheck && npm run build)
/Library/Frameworks/Python.framework/Versions/3.12/bin/python3 scripts/generate-test-fixtures.py
export PDFMASTER_CONTENT_FIXTURE="$PWD/work/parallel/content-edit/evidence/content-edit-qa.pdf"
export PDFMASTER_CONTENT_REPLACEMENT="$PWD/work/parallel/content-edit/evidence/replacement.png"
# qpdf 是外部工具；按自己的实际安装位置设置，或由 PATH 发现。
export PDFMASTER_QPDF=/absolute/path/to/qpdf
"$PDFMASTER_QPDF" --version
swift test
bash scripts/build-app.sh --qpdf "$PDFMASTER_QPDF"
codesign --verify --deep --strict 'dist/PDF Master.app'
```

先构建前端再测试，不能让前端构建与 WebKit 离线载入测试竞争同一个资源目录。夹具生成器拒绝覆盖已有文件；新 checkout 运行一次即可。未设置可选内容夹具会跳过其集成测试，但原生文字测试仍需生成夹具。用户故障文件 opt-in 默认跳过；不得把用户 PDF 或输出截图加入源码包。

## 外部引擎与发行差异

`build-app.sh` 生成新 App，不安装、不启动、不覆盖正式版，也不隐式复制外部工具。自行构建可通过 `PDFMASTER_QPDF` 运行；Finder 启动时环境可能不同。Release 内包含 qpdf 12.4.2/libqpdf 和静态整合的 libjpeg-turbo 3.2.0；它们是独立上游项目，不是缺失的 PDF Master 私有产品代码：

- qpdf 源码与许可：<https://github.com/qpdf/qpdf/tree/v12.4.2>
- libjpeg-turbo 源码与许可：<https://github.com/libjpeg-turbo/libjpeg-turbo/tree/3.2.0>

重做含引擎发行包时，将独立构建的 qpdf 放入 `Contents/Resources/Engines/qpdf/bin/qpdf`，libqpdf 放入其 `../lib/`，保留完整上游许可。部署目标 arm64/macOS14；qpdf 使用系统 native crypto、静态 JPEG。可执行文件和 libqpdf 的非系统动态依赖必须在该目录内闭合，使用 `otool -L` 核对，再对完整 App 重新 ad-hoc 签名并严格验证。不复制本机 Homebrew 路径依赖而冒称可携带。

Release 原生程序与本机正式版同功能源码，但构建路径中性化、公开画布资源及签名不同；不承诺不同工具链产出相同哈希。未获 Developer ID 签名或 Apple 公证，未完成其他 Mac 的安装验收。外部 Python 运行时不随 Release。

## 发布清单检查

```sh
python3 scripts/audit-source.py
python3 scripts/test-audit-source.py
shasum -a 256 -c SOURCE-MANIFEST.sha256
```

源码清单列出所有公开项目文件，不包括自身、依赖缓存、生成画布、构建输出、临时夹具、Git 历史、用户数据或私有研究素材。扫描是启发式检查，不是安全认证。
