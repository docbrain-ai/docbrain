#!/usr/bin/env python3
"""The release rule of workload_wiring_test.py: the release is one version in Chart.yaml (version and appVersion),
in every Cargo.toml and, when a tag is given, in the tag.

Each case writes a Chart.yaml and the Cargo.toml files to a scratch folder and runs the rule's own function on them;
then the wiring test itself runs once with a tag that names another version and once with another Cargo.toml, so the
options reach the rule.

Run: python3 helm/docbrain/tests/release_version_test.py   (from anywhere; helm, PyYAML, Python 3.11 or later)
Exit 0: every case held. Exit 1: a case broke. Exit 2: a tool is missing (never a pass).
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
WIRING = os.path.join(HERE, "workload_wiring_test.py")

# (case, Chart.yaml version, appVersion, [workspace version per Cargo.toml], tag, the rule fails)
CASES = [
    ("chart version behind appVersion and Cargo.toml", "1.6.5", "1.6.6", ["1.6.6"], None, True),
    ("chart version and appVersion behind Cargo.toml", "1.6.5", "1.6.5", ["1.6.6"], None, True),
    ("one version everywhere", "1.6.6", "1.6.6", ["1.6.6"], None, False),
    ("one version, a second Cargo.toml and the tag", "1.6.7", "1.6.7", ["1.6.7", "1.6.7"], "v1.6.7", False),
    ("a second Cargo.toml behind", "1.6.7", "1.6.7", ["1.6.7", "1.6.6"], None, True),
    ("the tag names a later version", "1.6.6", "1.6.6", ["1.6.6", "1.6.6"], "v1.6.7", True),
    ("the tag names an earlier version", "1.6.7", "1.6.7", ["1.6.7"], "v1.6.6", True),
    ("an lts tag names its version", "1.5.11", "1.5.11", ["1.5.11", "1.5.11"], "lts-v1.5.11", False),
    ("an lts tag that names another version", "1.5.11", "1.5.11", ["1.5.11"], "lts-v1.5.12", True),
    ("a tag with no v", "1.6.7", "1.6.7", ["1.6.7"], "1.6.7", True),
    ("an empty tag", "1.6.7", "1.6.7", ["1.6.7"], "", True),
    ("a Cargo.toml with no workspace version", "1.6.7", "1.6.7", ["1.6.7", ""], None, True),
]


def write_case(folder, chart_version, app_version, workspace_versions):
    chart = os.path.join(folder, "Chart.yaml")
    with open(chart, "w") as fh:
        fh.write(f"apiVersion: v2\nname: docbrain\nversion: {chart_version}\nappVersion: \"{app_version}\"\n")
    tomls = []
    for n, version in enumerate(workspace_versions):
        path = os.path.join(folder, f"checkout{n}", "Cargo.toml")
        os.makedirs(os.path.dirname(path))
        package = f"[workspace.package]\nversion = \"{version}\"\n" if version else "[workspace.package]\n"
        with open(path, "w") as fh:
            fh.write(f"[workspace]\nmembers = []\n\n{package}")
        tomls.append(path)
    return chart, tomls


def main():
    sys.path.insert(0, HERE)
    import workload_wiring_test as wiring
    broken = []
    for name, chart_version, app_version, workspace_versions, tag, should_fail in CASES:
        with tempfile.TemporaryDirectory(prefix="release-rule-") as folder:
            chart, tomls = write_case(folder, chart_version, app_version, workspace_versions)
            found, problems = wiring.release_versions(chart, tomls, tag)
        if bool(problems) != should_fail:
            broken.append(f"{name}: expected {'a failure' if should_fail else 'a pass'}, got {problems or found}")
    # The options reach the rule: the wiring test itself, on this repository, with a tag that names another version,
    # then with another Cargo.toml that names another version. Each must break the release rule.
    with tempfile.TemporaryDirectory(prefix="release-rule-") as folder:
        _, (other,) = write_case(folder, "0", "0", ["0.0.1"])
        for name, extra in (("--tag v0.0.1", ["--tag", "v0.0.1"]), ("--cargo-toml at 0.0.1", ["--cargo-toml", other])):
            run = subprocess.run([sys.executable, "-I", WIRING, *extra], capture_output=True, text=True)
            if run.returncode == 2:
                print(f"CANNOT RUN: the wiring test could not run:\n{run.stderr}", file=sys.stderr)
                sys.exit(2)
            if run.returncode != 1 or "[release]" not in run.stdout or "0.0.1" not in run.stdout:
                broken.append(f"wiring test {name}: expected a [release] failure naming 0.0.1, got exit "
                              f"{run.returncode}:\n{run.stdout}{run.stderr}")
    if broken:
        print(f"FAIL: {len(broken)} release rule case(s) broke")
        for b in broken:
            print("  " + b)
        sys.exit(1)
    print(f"PASS: the release rule holds over {len(CASES)} cases, and the wiring test's --tag and --cargo-toml reach it")


if __name__ == "__main__":
    try:
        import yaml  # noqa: F401  (the rule reads Chart.yaml with it)
        import tomllib  # noqa: F401  (and Cargo.toml with this)
    except ImportError as e:
        print(f"CANNOT RUN: {e} (PyYAML and Python 3.11 or later)", file=sys.stderr)
        sys.exit(2)
    main()
