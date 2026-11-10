#!/usr/bin/env python3
"""Conservative source guard, not a substitute for native foreground acceptance.

Tests must explicitly gate visible/native transport operations and inject hidden
AppDelegate presentation. Private pasteboards are required for clipboard writes.
This checks known entry points; it does not claim to infer arbitrary call graphs.
"""
import pathlib
import re
import sys


def code_only(source):
    # Preserve offsets while blanking comments and Swift string literals.
    pattern = r'//[^\n]*|/\*[\s\S]*?\*/|"""[\s\S]*?"""|"(?:\\.|[^"\\])*"'
    return re.sub(pattern, lambda match: re.sub(r'[^\n]', ' ', match.group()), source)


def functions(source):
    code = code_only(source)
    for match in re.finditer(r'\bfunc\s+(\w+)\s*\([^{}]*?\)[^{}]*?\{', code):
        depth, end = 1, match.end()
        while end < len(code) and depth:
            depth += (code[end] == '{') - (code[end] == '}')
            end += 1
        yield match.group(1), match.end(), code[match.end():end - 1]


RISKS = re.compile(
    r'\.(?:orderFront|orderFrontRegardless|makeKeyAndOrderFront|makeKeyWindow|makeMainWindow|'
    r'setActivationPolicy|beginSheetModal|beginSheet|runModal|showWindow)\s*\('
    r'|\bNSApp\.(?:activate|run)\s*\('
    r'|\bNSWorkspace\.shared\.(?:open|launchApplication)\s*\('
    r'|\bPDFWatermarkDependencies\s*\(\s*\)'
    r'|\.presentTextReplacement\s*\('
    r'|\bplus\.performClick\s*\('
    r'|\bdashboard\.perform\s*\('
    r'|\bwithContext\s*\(\s*presentation\s*:\s*\.interactive'
    r'|\bInvokeAEEventHandlerUPP\s*\('
)


def audit(source, name='<source>'):
    failures = []
    code = code_only(source)
    if re.search(r'NSPasteboard\s*\.\s*general\b', code):
        failures.append(f'{name}: tests must never use the global pasteboard')
    for function, offset, body in functions(source):
        # A conditional/caught/nested gate is not a top-level skip guarantee.
        gate = re.match(r'\s*try\s+NativeTestEnvironment\.requireForeground\s*\(', body)
        native_scopes = []
        for context in re.finditer(r'\btry\s+withNativeEventContext\s*\{', body):
            depth, end = 1, context.end()
            while end < len(body) and depth:
                depth += (body[end] == '{') - (body[end] == '}')
                end += 1
            native_scopes.append((context.end(), end - 1))
        for risk in RISKS.finditer(body):
            if not gate and not any(start <= risk.start() < end for start, end in native_scopes):
                line = source.count('\n', 0, offset + risk.start()) + 1
                failures.append(f'{name}:{line}: {function} requires an explicit foreground gate')
        for constructor in re.finditer(r'\bAppDelegate\s*\(', body):
            tail = body[constructor.end():]
            if not re.match(r'\s*controller\s*:', tail):
                failures.append(f'{name}: {function} must inject AppDelegate presentation')
            elif not re.search(r'\bpresentation\s*:', tail.split('\n', 1)[0]):
                failures.append(f'{name}: {function} must inject AppDelegate presentation')
            elif re.search(r'\bpresentation\s*:\s*\.interactive', tail.split('\n', 1)[0]) and not gate:
                failures.append(f'{name}: {function} requires a gate for interactive presentation')
        # Object copy tests must inject a private board, not rely on .shared.
        if re.search(r'\.(?:copy|cut|paste)\s*\(\s*nil\s*\)', body):
            if 'DocumentClipboard(pasteboard:' not in body:
                failures.append(f'{name}: {function} must inject a private clipboard')
    return failures


def audit_tree(root):
    return [failure for path in sorted((root / 'Tests').rglob('*.swift'))
            for failure in audit(path.read_text(), str(path.relative_to(root)))]


if __name__ == '__main__':
    root = pathlib.Path(__file__).resolve().parent.parent
    failures = audit_tree(root)
    print('\n'.join(failures) if failures else 'Background test source guard: PASS')
    sys.exit(bool(failures))
