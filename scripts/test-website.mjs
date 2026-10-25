import { readFileSync, existsSync } from 'node:fs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
const root = new URL('../docs/', import.meta.url);
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
test('homepage pins the verified public installer and labels limitations', () => {
  if (!existsSync(new URL('index.html', root))) return assert.fail('missing download homepage');
  const html = readFileSync(new URL('index.html', root), 'utf8');
  const release = JSON.parse(readFileSync(new URL('release.json', root), 'utf8'));
  assert.equal(release.version, 'V0.1.19');
  assert.equal(release.build, 41);
  assert.equal(release.sha256, 'ad3b298930d90048e76ae2e960fd9a757f2369236d1f660826ca50ea61a0d18d');
  assert.ok(html.includes(release.download));
  assert.ok(html.includes(release.checksum));
  assert.ok(html.includes('未获 Apple 公证'));
  assert.ok(html.includes('Python'));
  assert.ok(html.includes('对应项目源码已公开'));
  assert.equal(release.sourceVersion, 'V0.1.19/build41');
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
