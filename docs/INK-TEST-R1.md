# V0.1.19/build41 源码修订 r1：两项墨迹测试路径修复

## 根因与对照

公开副本曾在两项原生交互测试中出现PDFKit `InvalidTransition`，而正式工作区通过。两项失败测试分别在拖动画笔和选中墨迹线宽预览中把合成截图写到源码相对路径`docs/verification/`，但公开源码并不提供该私人证据目录。

在未修改产品或测试代码时，缺目录可稳定复现14项墨迹专项中的2项失败；只创建该目录，14项即全通过，并产生两张预期的合成截图。再次移走目录，改用UUID系统临时文件后14项同样通过。该对照支持失败由测试输出路径触发，PDFKit异常是失败清理过程中呈现的次生症状；不能据此断言其他PDFKit异常都是目录原因。

## 修复

- `InkInteractionTests.testNativeDragChangesVisibleInkBeforeReleaseAndCommitsOnlyOnce`
- `InkStyleBarTests.testNativeSelectedInkSliderPreviewsPixelsAndReleaseCommitsExactlyOnce`

两者均改为`FileManager.default.temporaryDirectory`加UUID命名，写后逐字节读回核对，`defer`清理。原有像素预览、模型/呈现层、按下拖动松开、只提交一次、撤销、宽度及保存重开断言不变。没有用skip、吞异常或删断言获得通过；不改墨迹产品实现。

## 验证

从上一公有main完整归档恢复全新独立目录，应用两项修复。先按`BUILD-build41.md`完成前端构建与合成内容夹具生成，再显式配置可执行且依赖闭合的qpdf。未创建`docs/verification/`，未复制私人开发工作目录或账户数据。

墨迹专项14通过；完整Swift测试780通过、0跳过、0失败（Desk547、Core233），包括本机显式opt-in清晰度测试。私有opt-in文件及截图不分发，别人未配置时仅该项应明确跳过。保留并行官网更新后，其最新3项、隐私扫描器3项通过；前端类型检查/生产构建及Swift发行构建通过。现有框架诊断及打包体积警告保留，不伪称日志无警告。

## 发布与安装边界

本次修订不修改产品源码/资源/版本或已签名程序；不是另一个墨迹算法修复。产品仍为V0.1.19/build41，已安装版本无需重装相同二进制。r1标签附同一已发布安装包（SHA256 `ad3b298930d90048e76ae2e960fd9a757f2369236d1f660826ca50ea61a0d18d`）和更新完整源码/校验文件，旧标签及附件保留。

不纳入并行进行中的原生内容引擎/商店适配，不拿未完成实现覆盖已验收安装。仍为ad-hoc签名、未Apple公证的预发布；外部Python运行时、其他Mac兼容性、真实付费API及历史Square崩溃等边界不因测试路径修复消失。
