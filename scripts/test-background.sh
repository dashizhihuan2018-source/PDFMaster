#!/bin/bash
# Background by default. Foreground validation is a separately authorized run:
# PDFMASTER_TEST_ALLOW_FOREGROUND=1 bash scripts/test-background.sh --filter ...
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
python3 scripts/check-background-tests.py
export PDFMASTER_TEST_ALLOW_FOREGROUND="${PDFMASTER_TEST_ALLOW_FOREGROUND:-0}"
if [[ "$PDFMASTER_TEST_ALLOW_FOREGROUND" == "1" ]]; then
    echo 'Explicit foreground opt-in: native windows/dialogs may be displayed.' >&2
else
    echo 'Background tests: foreground-dependent cases will be reported as skipped.' >&2
fi
exec swift test "$@"
