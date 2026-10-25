import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
const root = new URL('../', import.meta.url);
test('release smoke uses free standard public macOS runners and read-only permissions', () => {
  const workflow = readFileSync(new URL('.github/workflows/release-smoke.yml', root), 'utf8');
  assert.match(workflow, /contents: read/);
  assert.match(workflow, /macos-14, macos-15/);
  assert.ok(!/-xlarge|-large|secrets\./.test(workflow));
  assert.match(workflow, /timeout-minutes: 8/);
});
test('smoke verifies original public bytes and does not change macOS protection or signatures', () => {
  const shell = readFileSync(new URL('scripts/ci-release-smoke.sh', root), 'utf8');
  const swift = readFileSync(new URL('scripts/ci-release-smoke.swift', root), 'utf8');
  for (const text of ['shasum -a 256', 'codesign --verify --deep --strict', 'hdiutil verify', 'env -i PATH=/usr/bin:/bin', 'spctl --status']) assert.ok(shell.includes(text), text);
  assert.ok(swift.includes('NSWorkspace.shared.open'));
  assert.ok(swift.includes('app.terminate()'));
  assert.ok(swift.includes('--fixture-only'));
  assert.ok(!/kill\s+-9|forceTerminate|spctl\s+--(?:master-disable|global-disable)|xattr\s+-[a-z]*d|codesign\s+--force|sudo\s/.test(shell + swift));
  assert.ok(shell.includes('NOT a browser-quarantine or first-trust acceptance test'));
});
