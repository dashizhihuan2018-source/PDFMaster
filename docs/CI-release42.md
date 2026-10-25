# V0.1.20 公开包：独立 macOS 启动检查

2026-10-26，使用公有仓库的免费标准 GitHub 托管运行器。未使用付费 larger runner、账户密钥、用户 PDF 或私人样本。

- [完整运行记录](https://github.com/dashizhihuan2018-source/PDFMaster/actions/runs/40233164266)：success。
- 测试脚本提交：`8c01d1c819b3c51a86db93b23a2da9922fe4a37c`。
- 软件包仍为 `v0.1.20 / build42`，不是测试时重新构建的程序。
- 下载 SHA-256：`309884f872f9520a0ef85aea69335075c549528a63e056236118986b2c8755c8`。

| 运行器 | 实际系统 | 架构 | 安全评估状态 | 结果 |
| --- | --- | --- | --- | --- |
| macos-14 | 14.8.9 / 23J631 | arm64 | assessments enabled | PASS |
| macos-15 | 15.7.9 / 24G830 | arm64 | assessments enabled | PASS |

## 实际检查了什么

1. 从公开 Release 下载 DMG，核对哈希、磁盘镜像完整性、卷图标、build42 和 arm64 身份。
2. 原样复制包，严格深度签名校验；没有重签、修改安装包或更改安全设置。
3. PATH 仅含系统目录时运行包内 qpdf 12.4.2；用它检查现场生成的、只含自写文字的一页 PDF。
4. 通过 Launch Services 向下载 App 发送打开该样本的请求，回读真实启动 bundle，确认应用保持运行 10 秒。
5. 请求应用正常退出并确认退出，不强杀。测试副本仅存在于随后回收的测试 VM，不在用户本机留下重复 App。

## 不能从绿色检查推导什么

下载工具为 curl，不是浏览器；未验证浏览器添加的 quarantine 属性和首次信任弹窗。安全评估启用不等于本次触发并通过了完整门禁检查。没有截图验收、逐功能界面操作或在 VM 中编辑保存重开；这些检查不能用“进程持续运行”代替。

此包仍为 ad-hoc、未获 Apple 公证。测试不构成 Apple 背书、安全认证、所有 Mac 兼容承诺或无需首次确认的保证。需要了解首次安装及风险时，阅读[安装教程](guide.html#install)和 [Apple 官方安全说明](https://support.apple.com/zh-cn/102445)。
