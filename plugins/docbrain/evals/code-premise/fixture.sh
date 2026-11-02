#!/usr/bin/env bash
set -euo pipefail
mkdir -p src tests
printf 'pub fn run<F: FnMut() -> bool>(mut op: F) -> u32 {\n    const MAX_ATTEMPTS: u32 = 5;\n    let mut attempts = 0;\n    while attempts < MAX_ATTEMPTS {\n        attempts += 1;\n        if op() {\n            break;\n        }\n    }\n    attempts\n}\n' > src/retry.rs
printf '#[test]\nfn stops_after_five_attempts() {\n    let attempts = widgets::retry::run(|| false);\n    assert_eq!(attempts, 5);\n}\n' > tests/retry.rs
git init -q && git remote add origin https://github.com/acme/widgets.git
git add -A && git -c user.name=eval -c user.email=eval@example.test commit -qm "retry loop"
