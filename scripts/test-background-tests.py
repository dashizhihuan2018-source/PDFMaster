#!/usr/bin/env python3
import importlib.util
import pathlib
import os
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('background_guard', ROOT / 'scripts/check-background-tests.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class BackgroundGuardTests(unittest.TestCase):
    def test_rejects_ungated_window_and_transport(self):
        for call in ['window.orderFront(nil)', 'NSApp.activate(ignoringOtherApps: true)',
                     'NSApp.setActivationPolicy(.regular)', 'alert.beginSheetModal(for: window)',
                     'NSApp.run()', 'plus.performClick(nil)']:
            self.assertTrue(guard.audit('func testUnsafe() throws { ' + call + ' }'), call)

    def test_requires_gate_before_effect_not_after_it(self):
        self.assertTrue(guard.audit('func testUnsafe() throws { window.orderFront(nil); try NativeTestEnvironment.requireForeground() }'))
        self.assertFalse(guard.audit('func testOptIn() throws { try NativeTestEnvironment.requireForeground(); window.orderFront(nil) }'))

    def test_conditional_gate_and_expired_native_context_do_not_authorize_effect(self):
        self.assertTrue(guard.audit('func testUnsafe() throws { if flag { try NativeTestEnvironment.requireForeground() }; window.orderFront(nil) }'))
        self.assertTrue(guard.audit('func testUnsafe() throws { try withNativeEventContext { _ in }; window.orderFront(nil) }'))

    def test_ignores_comments_and_string_mentions(self):
        self.assertFalse(guard.audit('func testSafe() { // window.orderFront(nil)\n let s = "NSApp.activate()" }'))

    def test_requires_explicit_delegate_presentation(self):
        self.assertTrue(guard.audit('func testUnsafe() { let d = AppDelegate(controller: shell) }'))
        self.assertFalse(guard.audit('func testSafe() { let d = AppDelegate(controller: shell, presentation: .background) }'))
        self.assertTrue(guard.audit('func testUnsafe() { let d = AppDelegate(controller: shell, presentation: .interactive) }'))

    def test_global_clipboard_is_never_allowed(self):
        self.assertTrue(guard.audit('func testUnsafe() { NSPasteboard.general.clearContents() }'))
        self.assertTrue(guard.audit('func testUnsafe() { c.pdfView.copy(nil) }'))
        self.assertFalse(guard.audit('func testSafe() { c.clipboard = DocumentClipboard(pasteboard: pb); c.pdfView.copy(nil) }'))

    def test_repository_has_no_unguarded_known_effects(self):
        self.assertEqual(guard.audit_tree(ROOT), [])

    def run_wrapper(self, root=ROOT, foreground=None):
        with tempfile.TemporaryDirectory() as directory:
            swift = pathlib.Path(directory) / 'swift'
            swift.write_text('#!/bin/sh\nprintf "FLAG=%s\\n" "$PDFMASTER_TEST_ALLOW_FOREGROUND"\nprintf "<%s>\\n" "$@"\nexit 37\n')
            swift.chmod(0o755)
            environment = dict(os.environ, PATH=directory + os.pathsep + os.environ['PATH'])
            environment.pop('PDFMASTER_TEST_ALLOW_FOREGROUND', None)
            if foreground is not None:
                environment['PDFMASTER_TEST_ALLOW_FOREGROUND'] = foreground
            return subprocess.run(['bash', str(root / 'scripts/test-background.sh'), '--filter', 'Safe Tests'],
                                  env=environment, text=True, capture_output=True)

    def test_wrapper_defaults_to_background_forwards_arguments_and_exit_status(self):
        result = self.run_wrapper()
        self.assertEqual(result.returncode, 37)
        self.assertIn('FLAG=0\n<test>\n<--filter>\n<Safe Tests>', result.stdout)
        self.assertIn('reported as skipped', result.stderr)

    def test_wrapper_only_warns_for_explicit_foreground_without_running_real_swift(self):
        result = self.run_wrapper(foreground='1')
        self.assertEqual(result.returncode, 37)
        self.assertIn('FLAG=1', result.stdout)
        self.assertIn('Explicit foreground opt-in', result.stderr)

    def test_wrapper_fails_closed_before_swift_on_unsafe_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / 'scripts').mkdir(); (root / 'Tests').mkdir()
            for name in ['test-background.sh', 'check-background-tests.py']:
                shutil.copyfile(ROOT / 'scripts' / name, root / 'scripts' / name)
            (root / 'Tests/Unsafe.swift').write_text('func testUnsafe() { window.orderFront(nil) }')
            result = self.run_wrapper(root=root)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn('FLAG=', result.stdout)


if __name__ == '__main__':
    unittest.main()
