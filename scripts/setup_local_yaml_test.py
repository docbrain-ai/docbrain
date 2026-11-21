#!/usr/bin/env python3
"""The setup wizard's config/local.yaml stays one valid YAML document, however often it runs.

The server and the ingest read config/local.yaml at start and stop on a repeated key, so a second run of
scripts/setup.sh must never write a key twice. This runs the real wizard (scripts/setup.sh) several times in a
scratch folder, with `docker` and `curl` replaced by stubs that start nothing (the docker stub logs what it was
asked), and after every run parses the file with a YAML loader that refuses a duplicated key, as the server's
parser does. It also holds that a run which refuses to write never says "Configuration saved." nor starts the
stack, that a rewrite first keeps the previous file as a backup of its own (config/local.yaml.bak.<UTC time>) and
never replaces or writes through one already there, that a backup or a write that fails
stops the run before it says it kept or saved anything, and that the providers and the source the wizard uses for
an answer that is none of its choices are the ones it names.

Run: python3 scripts/setup_local_yaml_test.py   (from anywhere; nothing starts, nothing is downloaded)
Needs bash and PyYAML. A missing tool is a failure (exit 2), never a pass. Exit 0: every case held.
"""
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

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
        self.docker_log = os.path.join(root, "docker.log")
        for tool in ("docker", "curl"):
            path = os.path.join(self.stubs, tool)
            with open(path, "w") as f:
                f.write("#!/bin/sh\n" + (f"echo \"$*\" >> '{self.docker_log}'\n" if tool == "docker" else "")
                        + "exit 0\n")
            os.chmod(path, 0o755)
        self.cp_log = os.path.join(root, "cp.log")
        self.local = os.path.join(root, "config", "local.yaml")
        self.runs = 0

    def run(self, case, answers, expect=0):
        """Run the wizard; `expect` is its exit code. A refused write (1) must not say it saved nor start."""
        self.runs += 1
        if os.path.exists(self.docker_log):
            os.remove(self.docker_log)
        env = {"PATH": f"{self.stubs}:/usr/bin:/bin", "HOME": self.root, "LC_ALL": "C"}
        p = subprocess.run(["bash", os.path.join(self.root, "scripts", "setup.sh")], cwd=self.root, env=env,
                           input="\n".join(answers) + "\n", capture_output=True, text=True, timeout=120)
        out = p.stdout + p.stderr
        started = os.path.exists(self.docker_log) and "compose up" in open(self.docker_log).read()
        if p.returncode != expect:
            fail(case, f"the wizard exited {p.returncode}, want {expect}: {out[-400:]}")
        if expect == 0 and not started:
            fail(case, "the wizard finished without starting the stack (no `docker compose up`)")
        if expect != 0 and "Configuration saved." in out:
            fail(case, "the wizard said \"Configuration saved.\" after it refused to write config/local.yaml")
        if expect != 0 and started:
            fail(case, "the wizard started the stack after it refused to write config/local.yaml")
        return out

    def failing_backup(self, on):
        """With `on`, a `cp` stub refuses any copy to the backup's name (and logs that it did); other copies run."""
        path = os.path.join(self.stubs, "cp")
        if not on:
            os.remove(path)
            return
        with open(path, "w") as f:
            f.write("#!/bin/sh\nfor last; do :; done\n"
                    f"case \"$last\" in *config/local.yaml.bak*) echo \"$*\" >> '{self.cp_log}'; exit 1;; esac\n"
                    "exec /bin/cp \"$@\"\n")
        os.chmod(path, 0o755)

    def env(self):
        with open(os.path.join(self.root, ".env")) as f:
            return dict(line.split("=", 1) for line in f.read().splitlines() if "=" in line and line[0] != "#")

    def backups(self):
        """{name: text, or None for a link or a non-file} for every config/local.yaml.bak* there."""
        return {os.path.basename(p): read_or_none(p) for p in glob.glob(self.local + ".bak*")}

    def kept(self, case, out):
        """The backup the run says it kept, or None (a failure is recorded) when it names none."""
        m = re.search(r"Kept the previous config/local\.yaml as (config/local\.yaml\.bak\.\S+)\.$", out, re.M)
        if not m:
            fail(case, "the run did not say which backup it kept")
            return None
        return os.path.join(self.root, m.group(1))

    def read(self):
        with open(self.local) as f:
            return f.read()

    def write(self, text):
        with open(self.local, "w") as f:
            f.write(text)


def read_or_none(path):
    """The file's text, or None when there is no regular file there (a test must fail, not crash)."""
    return open(path).read() if os.path.isfile(path) and not os.path.islink(path) else None


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
        #    The file it replaced is kept as a backup of its own, and the run names it.
        before = s.read()
        out = s.run("second run", ["y", "3", "3", *CONF, "tok-2", "DOCS,ENG"])
        text, _ = check("second run", s, {"confluence.api_token": "tok-2"})
        if blocks(text) != (1, 1):
            fail("second run", f"want one marked block, found {blocks(text)}")
        kept = s.kept("second run", out)
        if kept and read_or_none(kept) != before:
            fail("second run", f"{os.path.basename(kept)} is not the file the run replaced")

        # 2b. Answers with characters YAML would otherwise read as syntax stay the strings typed.
        token = "it's: #not-a-comment &x *y"
        s.run("yaml characters", ["y", "3", "3", *CONF, token, " DOCS "])
        text, _ = check("yaml characters", s, {"confluence.api_token": token, "confluence.space_keys": "DOCS"})

        # 3. The same answers once more: the file is byte for byte the same, and nothing is backed up.
        earlier = s.backups()
        out = s.run("same answers", ["y", "3", "3", *CONF, token, " DOCS "])
        if s.read() != text:
            fail("same answers", "a run with the same answers changed config/local.yaml")
        if "config/local.yaml.bak" in out or s.backups() != earlier:
            fail("same answers", "a run that changed nothing wrote a backup")

        # 3b. The backup keeps the file's mode (it holds credentials), and a backup or a link already at a backup's
        #     name is neither replaced nor written through: links sit at every free name the next 30 seconds would
        #     give, suffixes -2 to -9 included, so whatever second the run lands in, the first names it tries are
        #     taken by a link or by an earlier backup, and it must go past them all.
        os.chmod(s.local, 0o600)
        outside = os.path.join(root, "outside.txt")
        with open(outside, "w") as f:
            f.write("not the wizard's\n")
        now = time.time()
        stamps = [f"{s.local}.bak.{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime(now + d))}" for d in range(31)]
        names = [f"{stamp}{suffix}" for stamp in stamps for suffix in ["", *(f"-{n}" for n in range(2, 10))]]
        earlier = s.backups()
        planted = [n for n in names if not os.path.lexists(n)]  # a name an earlier run took keeps its backup
        for link in planted:
            os.symlink(outside, link)
        before = s.read()
        out = s.run("backup mode and link", ["y", "3", "3", *CONF, "tok-mode", " DOCS "])
        kept = s.kept("backup mode and link", out)
        if open(outside).read() != "not the wizard's\n":
            fail("backup mode and link", "the backup was written through a link at its name")
        if any(not os.path.islink(link) for link in planted):
            fail("backup mode and link", "a link at a backup's name was replaced")
        for name, text in earlier.items():
            if s.backups().get(name) != text:
                fail("backup mode and link", f"{name}, kept by an earlier run, was replaced or changed")
        if kept in planted:
            fail("backup mode and link", f"the run says it kept {os.path.basename(kept)}, where a link sits")
        elif kept and read_or_none(kept) != before:
            fail("backup mode and link", f"{os.path.basename(kept)} is not a copy of the file the run replaced")
        elif kept and os.stat(kept).st_mode & 0o777 != 0o600:
            fail("backup mode and link", f"the backup's mode is {oct(os.stat(kept).st_mode & 0o777)}, want 0o600")
        for link in planted:
            os.remove(link)
        s.run("token back", ["y", "3", "3", *CONF, token, " DOCS "])

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

        # 5b. The file is damaged (emptied) and the wizard runs again: the backups earlier runs kept are all still
        #     there and unchanged, so the hand-written lines survive in one of them, and the damaged file gets a
        #     backup of its own.
        earlier, good = s.backups(), s.read()
        s.write("")
        out = s.run("after damage", ["y", "3", "4", "acme/docs", "ghp_y", "main"])
        kept = s.kept("after damage", out)
        now_there = s.backups()
        for name, text in earlier.items():
            if now_there.get(name) != text:
                fail("after damage", f"{name}, kept by an earlier run, was replaced or changed")
        if not any(t and "archived_title_patterns" in t for t in now_there.values()):
            fail("after damage", "no backup holds the hand-written lines any more")
        if kept and read_or_none(kept) != "":
            fail("after damage", f"{os.path.basename(kept)} is not the damaged file the run replaced")
        s.write(good)  # put back from the backup, as the user would; the cases below build on these lines

        # 6. A sources: section written by hand: choosing GitHub would write sources: twice, so the wizard writes
        #    nothing and says what to add.
        s.run("sample docs", ["y", "3", "1"])
        s.write(s.read() + "\nsources:\n  jira:\n    base_url: https://jira.example.org\n")
        before = s.read()
        out = s.run("hand-written sources", ["y", "3", "4", "acme/docs", "", ""], expect=1)
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
        s.run("unterminated block", ["y", "3", "3", *CONF, "tok-3", "DOCS"], expect=1)
        if s.read() != before:
            fail("unterminated block", "the wizard changed a file whose block has no end line")
        check("unterminated block", s, {"confluence.base_url": "https://kept.example.org"})

        # 9. An answer that is no provider: the wizard says it uses Anthropic, and .env says so too (the template's
        #    own provider is another one). It then asks for the Anthropic key and the embedding provider.
        out = s.run("invalid provider", ["y", "9", "sk-ant-x", "1", "sk-x", "1"])
        env = s.env()
        if "Using Anthropic as default" not in out:
            fail("invalid provider", "the wizard did not say which provider it chose")
        for k, v in {"LLM_PROVIDER": "anthropic", "LLM_MODEL_ID": "claude-sonnet-5-5", "ANTHROPIC_API_KEY": "sk-ant-x",
                     "EMBED_PROVIDER": "openai", "EMBED_MODEL_ID": "text-embedding-3-small"}.items():
            if env.get(k) != v:
                fail("invalid provider", f".env has {k}={env.get(k)!r}, want {v!r}")

        # 10. The backup copy fails: the run says so, never that it kept a backup, leaves the file as it was, and
        #     neither says it saved nor starts (the copy runs inside an `if`, where `set -e` does not stop it). The
        #     file starts with no confluence: of its own (case 8 left one), so the run gets as far as the backup.
        s.write("freshness:\n  exclusion_rules:\n    archived_title_patterns: ['^Old ']\n")
        before = s.read()
        s.failing_backup(True)
        out = s.run("backup fails", ["y", "3", "3", *CONF, "tok-cp", "DOCS"], expect=1)
        s.failing_backup(False)
        if not os.path.exists(s.cp_log):
            fail("backup fails", "the cp stub never refused a copy, so this case tested nothing")
        if "Kept the previous" in out:
            fail("backup fails", "the run said it kept a backup when the copy failed")
        if "Could not keep the previous config/local.yaml" not in out:
            fail("backup fails", "the run did not say the backup failed")
        if s.read() != before:
            fail("backup fails", "the run changed config/local.yaml although it had no backup")

        # 10a. The same for the sample docs, whose call removes the wizard's block: the run ends on the wizard's own
        #      account of what it did not write, not on `set -e`.
        s.run("confluence before a failed removal", ["y", "3", "3", *CONF, "tok-ok", "DOCS"])
        before = s.read()
        s.failing_backup(True)
        out = s.run("removal backup fails", ["y", "3", "1"], expect=1)
        s.failing_backup(False)
        if ".env is saved, but" not in out:
            fail("removal backup fails", "the run did not say which settings it left unsaved")
        if s.read() != before:
            fail("removal backup fails", "the run changed config/local.yaml although it had no backup")

        # 10b. The write fails (a file the user cannot write): the run says so and neither saves nor starts. Root
        #      writes a read-only file anyway, so this one needs another user and says when it did not run.
        write_case = "not run (root writes a read-only file)"
        if os.geteuid() != 0:
            write_case = "run"
            before = s.read()
            os.chmod(s.local, 0o444)
            out = s.run("write fails", ["y", "3", "3", *CONF, "tok-ro", "DOCS"], expect=1)
            os.chmod(s.local, 0o600)
            if "Could not write config/local.yaml" not in out:
                fail("write fails", "the run did not say the write failed")
            if s.read() != before:
                fail("write fails", "config/local.yaml changed although the write failed")

        # 11. Item 9's twin one prompt lower: an embedding answer that is none of the choices. The wizard says it uses
        #     OpenAI, and .env says so (the template's own embedding provider is another one).
        out = s.run("invalid embedding", ["y", "1", "sk-ant-x", "9", "sk-emb", "1"])
        env = s.env()
        if "Invalid choice. Using OpenAI as default." not in out:
            fail("invalid embedding", "the wizard did not say which embedding provider it chose")
        for k, v in {"LLM_PROVIDER": "anthropic", "EMBED_PROVIDER": "openai", "EMBED_MODEL_ID": "text-embedding-3-small",
                     "OPENAI_API_KEY": "sk-emb"}.items():
            if env.get(k) != v:
                fail("invalid embedding", f".env has {k}={env.get(k)!r}, want {v!r}")

        # 12. A source answer that is none of the choices: the wizard says it uses the sample docs, and does what that
        #     choice does (an earlier run's source block goes, its lines kept in a backup), not leave it in silence.
        s.run("confluence before an invalid source", ["y", "3", "3", *CONF, "tok-src", "DOCS"])
        out = s.run("invalid source", ["y", "3", "9"])
        text, _ = check("invalid source", s, {}, absent=("confluence",))
        if "Invalid choice. Using the sample docs." not in out:
            fail("invalid source", "the wizard did not say which source it chose")
        if blocks(text) != (0, 0):
            fail("invalid source", f"the earlier run's source block is still there: {blocks(text)}")
        runs = s.runs

    if failures:
        print(f"FAIL: {len(failures)} case(s) broken")
        for f in failures:
            print(f"  {f}")
        return 1
    print(f"PASS: config/local.yaml stays one valid YAML document over {runs} wizard runs (write-failure case: "
          f"{write_case})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
