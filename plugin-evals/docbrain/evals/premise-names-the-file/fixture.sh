#!/usr/bin/env bash
set -euo pipefail
mkdir -p services/report vendor/toolkit
cat > services/report/build.py <<'P'
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "vendor"))

from toolkit.render import table


def build(rows):
    return table(rows, width=120)
P
: > vendor/toolkit/__init__.py
cat > vendor/toolkit/render.py <<'P'
def table(rows, width=80):
    """Render rows as a text table, each line cut at `width` columns."""
    lines = []
    for row in rows:
        lines.append(" | ".join(str(cell) for cell in row)[:width])
    return "\n".join(lines)
P
git init -q && git add -A && git -c user.name=eval -c user.email=eval@example.test commit -qm "weekly report: wider tables"
