import { readFileSync, existsSync } from 'node:fs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
const root = new URL('../docs/', import.meta.url);
test('PNG signature release records reuse, deletion boundaries and actual verification', () => {
  const report = readFileSync(new URL('SIGNATURES-V0.1.30.md', root), 'utf8');
  for (const phrase of ['右侧签名库', '重启后历史', '仅删除签名库副本', '不是数字证书签名', '取消', '没有二次确认', '未压力实测', '未获Apple公证']) assert.ok(report.includes(phrase), phrase);
  const guide = readFileSync(new URL('guide.html', root), 'utf8');
  assert.ok(guide.includes('导入 PNG'));
  assert.ok(guide.includes('删除'));
});
test('layout UI release states measured scope and visual acceptance limits', () => {
  const report = readFileSync(new URL('LAYOUT-UI-V0.1.28.md', root), 'utf8');
  for (const phrase of ['28点按钮', '1点蓝色轮廓', '240点', '886', '884通过', '2项私人样本跳过', '逐像素一致', 'VoiceOver']) assert.ok(report.includes(phrase), phrase);
  const release = JSON.parse(readFileSync(new URL('release.json', root), 'utf8'));
  assert.equal(release.version, 'V0.1.32');
  assert.equal(release.build, 54);
});
test('layout sidebar guide preserves real scrolling and states split limits', () => {
  const report = readFileSync(new URL('LAYOUT-SIDEBAR-V0.1.27.md', root), 'utf8');
  for (const phrase of ['固定左栏', '单独显示首页', '显示 → 连续滚动', '没有新增', '882', '2 跳过']) assert.ok(report.includes(phrase), phrase);
  const guide = readFileSync(new URL('guide.html', root), 'utf8');
  assert.ok(guide.includes('显示 → 连续滚动'));
});
test('native outline release distinguishes whitespace from missing data', () => {
  const report = readFileSync(new URL('OUTLINE-V0.1.24.md', root), 'utf8');
  for (const phrase of ['54点', '28点', '未命名', '不改原PDF', '整栏完全空白', '用户接受']) assert.ok(report.includes(phrase), phrase);
});
test('native sidebar release documents persistent width and honest verification', () => {
  const guide = readFileSync(new URL('guide.html', root), 'utf8');
  const report = readFileSync(new URL('SIDEBAR-WIDTH-V0.1.23.md', root), 'utf8');
  assert.ok(guide.includes('拖动调宽左栏'));
  assert.ok(guide.includes('176–480'));
  assert.ok(report.includes('176→324→280'));
  assert.ok(report.includes('noWindowsAvailable'));
  assert.ok(report.includes('微信接收端仍未实际'));
});
test('download homepage exists', () => {
  assert.ok(existsSync(new URL('index.html', root)), 'missing download homepage');
});
test('homepage reuses campaign, poster, video and actual app icon', () => {
  const html = readFileSync(new URL('index.html', root), 'utf8');
  for (const phrase of ['自由创作，', '灵感无界。', '一个应用，三重自由', 'PDF 工具集', '无限画布', '无边记']) assert.ok(html.includes(phrase), phrase);
  for (const file of ['pdfmaster-poster.png', 'pdfmaster-demo.mp4', 'pdfmaster-icon.png']) {
    assert.ok(html.includes('media/' + file), file);
    assert.ok(existsSync(new URL('media/' + file, root)), file);
  }
  assert.match(html, /<video\b[^>]*controls/);
  assert.match(html, /<link rel="icon"[^>]*pdfmaster-icon.png/);
  assert.ok(!/<video\b[^>]*autoplay/.test(html));
});
test('detailed feature discovery and locally hosted bold serif are shipped', () => {
  const html = readFileSync(new URL('index.html', root), 'utf8');
  assert.match(html, /class="feature-grid"/);
  assert.ok((html.match(/class="feature-card/g) ?? []).length >= 8);
  for (const file of ['features.html', 'guide.html', 'product.css', 'fonts/OFL.txt', 'fonts/SourceHanSerifCN-Bold.woff2']) assert.ok(existsSync(new URL(file, root)), file);
  const css = readFileSync(new URL('product.css', root), 'utf8');
  assert.match(css, /font-family:SourceHanSerifBold/); assert.match(css, /font-weight:700/);
});
test('first installation is actionable without globally disabling macOS protection', () => {
  const html = readFileSync(new URL('index.html', root), 'utf8');
  const guide = readFileSync(new URL('guide.html', root), 'utf8');
  assert.ok(html.includes('guide.html#install'));
  assert.ok(guide.includes('id="install"'));
  for (const phrase of ['隐私与安全性', '仍要打开', '可信来源', '未遭篡改', '已损坏', '将损坏', '系统管理员', '另一台 Mac', 'shasum -a 256']) assert.ok(guide.includes(phrase), phrase);
  assert.ok(guide.includes('https://support.apple.com/zh-cn/102445'));
  assert.ok(guide.includes('dc155075821b4bf79d9534040b0652b22add95c4562b09d22e2190fcfdded00f'));
  assert.ok(!/spctl\s+--(?:master-disable|global-disable)|xattr\s+-[a-z]*d|sudo\s/.test(html + guide));
});
test('homepage pins the verified public installer and labels limitations', () => {
  if (!existsSync(new URL('index.html', root))) return assert.fail('missing download homepage');
  const html = readFileSync(new URL('index.html', root), 'utf8');
  const release = JSON.parse(readFileSync(new URL('release.json', root), 'utf8'));
  assert.equal(release.version, 'V0.1.32');
  assert.equal(release.build, 54);
  assert.equal(release.sha256, 'dc155075821b4bf79d9534040b0652b22add95c4562b09d22e2190fcfdded00f');
  assert.ok(html.includes(release.download));
  assert.ok(html.includes(release.checksum));
  assert.ok(html.includes('未获 Apple 公证'));
  assert.ok(html.includes('Python'));
  assert.ok(html.includes('对应项目源码已公开'));
  assert.equal(release.sourceVersion, 'V0.1.32/build54');
  assert.equal(release.selfContainedContentEditing, true);
  assert.ok(html.includes('不再依赖外置 Python'));
  assert.ok(html.includes('不会上传'));
  assert.ok(html.includes('© 2026 Nankong'));
  assert.ok(!html.includes('完全自主知识产权'));
  assert.ok(!html.includes('/Users/'));
  assert.ok(!/<script[^>]+src=["']https?:/i.test(html));
  assert.ok(!/fonts\.(googleapis|gstatic)/i.test(html));
  assert.match(html, /name="viewport"/);
  assert.match(readFileSync(new URL('site.css', root), 'utf8'), /prefers-reduced-motion/);
  assert.match(readFileSync(new URL('site.css', root), 'utf8'), /@media/);
});

test('all local page and resource links resolve, including cross-page anchors', () => {
  const pages = ['index.html', 'features.html', 'guide.html'];
  for (const page of pages) {
    const html = readFileSync(new URL(page, root), 'utf8');
    for (const match of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
      if (/^(?:https?:|mailto:|data:)/.test(match[1])) continue;
      const link = new URL(match[1], new URL(page, root));
      if (link.pathname.endsWith('/')) link.pathname += 'index.html';
      assert.ok(existsSync(link), `${page}: ${match[1]}`);
      if (link.hash) {
        const target = readFileSync(link, 'utf8');
        assert.ok(target.includes(`id="${decodeURIComponent(link.hash.slice(1))}"`), `${page}: missing anchor ${match[1]}`);
      }
    }
    if (page !== 'index.html') {
      assert.ok(html.includes('V0.1.32 / build54'));
      assert.ok(!html.includes('尚未发布') && !html.includes('待发布预览'));
    }
  }
});
