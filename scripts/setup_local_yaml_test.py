#!/usr/bin/env python3
"""The setup wizard's config/local.yaml stays one valid YAML document, however often it runs.

The server and the ingest read config/local.yaml at start and stop on a repeated key, so a second run of
scripts/setup.sh must never write a key twice. This runs the real wizard (scripts/setup.sh) several times in a
scratch folder, with `docker` and `curl` replaced by stubs that start nothing, and after every run parses the
file with a YAML loader that refuses a duplicated key, as the server's parser does.

Run: python3 scripts/setup_local_yaml_test.py   (from anywhere; nothing starts, nothing is downloaded)
Needs bash and PyYAML. A missing tool is a failure (exit 2), never a pass. Exit 0: every case held.
"""
import os
import shutil
import subprocess
import sys
import tempfile

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
BEGIN = "# >>> written by scripts/setup.sh; running the wizard again replaces this block >>>"
END = "# <<< end of the block scripts/setup.sh writes <<<"

try:
    import yaml
except ImportError:
    print("FAIL: PyYAML is not installed; the test did not run")
    sys.exit(2)
if not shutil.which("bash"):
    print("FAIL: bash not found; the test did not run")
    sys.exit(2)


class StrictLoader(yaml.SafeLoader):
    """SafeLoader that refuses a key repeated in one mapping (the server's loader stops on one)."""


def _strict_mapping(loader, node, deep=False):
    seen = set()
    for key_node, _ in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in seen:
            raise yaml.constructor.ConstructorError(None, None, f"duplicated key {key!r}", key_node.start_mark)
        seen.add(key)
    return yaml.SafeLoader.construct_mapping(loader, node, deep)


StrictLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _strict_mapping)

failures = []

# The loader must itself refuse a repeated key, or every case below would pass on a broken file.
try:
    yaml.load("a: 1\nb:\n  c: 1\n  c: 2\n", Loader=StrictLoader)
    print("FAIL: the test's YAML loader accepts a repeated key; the test did not run")
    sys.exit(2)
except yaml.constructor.ConstructorError:
    pass


def fail(case, msg):
    failures.append(f"{case}: {msg}")


def parse(case, text):
    """The file as the server would read it: one document, no repeated key. None when it does not parse."""
    try:
        docs = list(yaml.load_all(text, Loader=StrictLoader))
    except yaml.YAMLError as e:
        fail(case, f"config/local.yaml does not parse: {e}")
        return None
    if len(docs) > 1:
        fail(case, f"config/local.yaml holds {len(docs)} YAML documents, not one")
        return None
    return (docs[0] if docs else None) or {}


class Scratch:
    """A scratch copy of the wizard and .env.example, with docker and curl stubbed out."""

    def __init__(self, root):
        self.root = root
        os.makedirs(os.path.join(root, "scripts"))
        shutil.copy(os.path.join(REPO, "scripts", "setup.sh"), os.path.join(root, "scripts", "setup.sh"))
        shutil.copy(os.path.join(REPO, ".env.example"), os.path.join(root, ".env.example"))
        self.stubs = os.path.join(root, "stubs")
        os.makedirs(self.stubs)
        for tool in ("docker", "curl"):
            path = os.path.join(self.stubs, tool)
            with open(path, "w") as f:
                f.write("#!/bin/sh\nexit 0\n")
            os.chmod(path, 0o755)
        self.local = os.path.join(root, "config", "local.yaml")
        self.runs = 0

    def run(self, case, answers):
        self.runs += 1
        env = {"PATH": f"{self.stubs}:/usr/bin:/bin", "HOME": self.root, "LC_ALL": "C"}
        p = subprocess.run(["bash", os.path.join(self.root, "scripts", "setup.sh")], cwd=self.root, env=env,
                           input="\n".join(answers) + "\n", capture_output=True, text=True, timeout=120)
        if p.returncode != 0:
            fail(case, f"the wizard exited {p.returncode}: {(p.stderr or p.stdout)[-400:]}")
        return p.stdout + p.stderr

    def read(self):
        with open(self.local) as f:
            return f.read()

    def write(self, text):
        with open(self.local, "w") as f:
            f.write(text)


def blocks(text):
    return text.count(BEGIN), text.count(END)


def check(case, s, want, absent=()):
    """Parse the file and compare the settings the wizard owns; `absent` keys must not be there."""
    text = s.read()
    data = parse(case, text)
    if data is None:
        return text, None
    for path, value in want.items():
        node = data
        for part in path.split("."):
            node = node.get(part) if isinstance(node, dict) else None
        if node != value:
            fail(case, f"{path} is {node!r}, want {value!r}")
    for key in absent:
        if key in data:
            fail(case, f"top-level {key!r} is still there: {data[key]!r}")
    return text, data


CONF = ["https://wiki.example.org/wiki", "someone@example.org"]


def main():
    with tempfile.TemporaryDirectory(prefix="setup-local-yaml-") as root:
        s = Scratch(root)

        # 1. First run: the template, then one Confluence block.
        s.run("first run", ["3", "3", *CONF, "tok-1", "DOCS,ENG"])
        text, _ = check("first run", s, {"confluence.api_token": "tok-1", "confluence.space_keys": "DOCS,ENG"})
        if blocks(text) != (1, 1):
            fail("first run", f"want one marked block, found {blocks(text)}")

        # 2. The same choice again with another token (the mistyped-token re-run): one confluence key, the new token.
        s.run("second run", ["y", "3", "3", *CONF, "tok-2", "DOCS,ENG"])
        text, _ = check("second run", s, {"confluence.api_token": "tok-2"})
        if blocks(text) != (1, 1):
            fail("second run", f"want one marked block, found {blocks(text)}")

        # 2b. Answers with characters YAML would otherwise read as syntax stay the strings typed.
        token = "it's: #not-a-comment &x *y"
        s.run("yaml characters", ["y", "3", "3", *CONF, token, " DOCS "])
        text, _ = check("yaml characters", s, {"confluence.api_token": token, "confluence.space_keys": "DOCS"})

        # 3. The same answers once more: the file is byte for byte the same.
        s.run("same answers", ["y", "3", "3", *CONF, token, " DOCS "])
        if s.read() != text:
            fail("same answers", "a run with the same answers changed config/local.yaml")

        # 4. Another source: GitHub with no token replaces the wizard's Confluence block and writes no token
        #    (a placeholder token would be sent to GitHub and refused).
        s.run("github", ["y", "3", "4", "acme/docs", "", ""])
        _, data = check("github", s, {"sources.github.code.repos": ["acme/docs"]}, absent=("confluence",))
        if data and "token" in ((data.get("sources") or {}).get("github") or {}):
            fail("github", "an empty token answer wrote a token line")

        # 5. Settings written by hand outside the block are kept, and the wizard's block is still replaced.
        s.write(s.read() + "\nfreshness:\n  exclusion_rules:\n    archived_title_patterns: ['^Old ']\n")
        s.run("hand-written kept", ["y", "3", "4", "acme/docs", "ghp_x", "main"])
        text, _ = check("hand-written kept", s, {"sources.github.code.repos": ["acme/docs:main"],
                                                 "sources.github.token": "ghp_x",
                                                 "freshness.exclusion_rules.archived_title_patterns": ["^Old "]})
        if blocks(text) != (1, 1):
            fail("hand-written kept", f"want one marked block, found {blocks(text)}")

        # 6. A sources: section written by hand: choosing GitHub would write sources: twice, so the wizard writes
        #    nothing and says what to add.
        s.run("sample docs", ["y", "3", "1"])
        s.write(s.read() + "\nsources:\n  jira:\n    base_url: https://jira.example.org\n")
        before = s.read()
        out = s.run("hand-written sources", ["y", "3", "4", "acme/docs", "", ""])
        if s.read() != before:
            fail("hand-written sources", "the wizard changed a file whose sources: section was written by hand")
        if "already has a sources: section" not in out:
            fail("hand-written sources", "the wizard did not say why it left the file unchanged")
        check("hand-written sources", s, {"sources.jira.base_url": "https://jira.example.org"})

        # 7. Sample docs removes the wizard's own block and keeps the rest.
        s.run("confluence again", ["y", "3", "3", *CONF, "tok-2", "DOCS"])
        s.run("sample docs again", ["y", "3", "1"])
        text, _ = check("sample docs again", s, {"sources.jira.base_url": "https://jira.example.org",
                                                 "freshness.exclusion_rules.archived_title_patterns": ["^Old "]},
                        absent=("confluence",))
        if blocks(text) != (0, 0):
            fail("sample docs again", f"the wizard's block is still there: {blocks(text)}")

        # 8. A start line whose end line was deleted by hand: its lines are kept, so its confluence: counts as
        #    written by hand, and choosing Confluence writes nothing rather than a second key.
        s.write(s.read() + f"\n{BEGIN}\nconfluence:\n  base_url: https://kept.example.org\n")
        before = s.read()
        s.run("unterminated block", ["y", "3", "3", *CONF, "tok-3", "DOCS"])
        if s.read() != before:
            fail("unterminated block", "the wizard changed a file whose block has no end line")
        check("unterminated block", s, {"confluence.base_url": "https://kept.example.org"})
        runs = s.runs

    if failures:
        print(f"FAIL: {len(failures)} case(s) broken")
        for f in failures:
            print(f"  {f}")
        return 1
    print(f"PASS: config/local.yaml stays one valid YAML document over {runs} wizard runs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
