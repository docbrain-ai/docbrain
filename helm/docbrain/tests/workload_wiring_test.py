#!/usr/bin/env python3
"""Wiring rules that must hold on every workload the chart renders, checked over `helm template` output.

Each rule lists every workload (and every ConfigMap and Secret) the chart renders, so a new workload or object
that arrives without the rule fails here instead of on a cluster:

  reads      every ConfigMap or Secret the chart renders is read by some pod (a Secret nobody reads is a
             setting that silently does nothing: the bootstrap admin key once was one), and every reference to
             a chart object resolves to one that is rendered, with the key it names.
  checksum   a pod carries checksum/<suffix> exactly for the chart objects (<fullname>-<suffix>) it reads whole
             (envFrom or a volume). Fewer, and a settings change never reaches it; more, and it restarts for
             nothing (the datastores restarted on every server setting change).
  keys       a single key read by secretKeyRef or configMapKeyRef carries no checksum, so each one is listed in
             KEY_READS with its reason; a new one fails until it is.
  restart    changing one object's content changes the pod template of exactly the pods that read it whole.
  overlay    when the chart's local.yaml renders, every pod that runs DocBrain with the chart's settings (it
             reads <fullname>-config whole) mounts it at /app/config/local.yaml.
  bootstrap  when the chart generates the first-login admin key, the server is given it.
  startup    OpenSearch's startup probe outlasts twice its slowest measured first boot.

Run: python3 helm/docbrain/tests/workload_wiring_test.py   (from anywhere; renders only, no cluster)
Needs helm and PyYAML. A missing tool is a failure (exit 2), never a pass. Exit 0: every rule held.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys

CHART = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
RELEASE = "wiring"
FULLNAME = "wiring-docbrain"
POD_KINDS = {"Deployment", "StatefulSet", "DaemonSet", "Job", "CronJob"}

# Single keys read by reference, and why a change to them needs no restart. (component, object suffix, key).
KEY_READS = {
    ("postgres", "secret", "POSTGRES_PASSWORD"):
        "used only when an empty data volume is first initialised; a restart would not change the password",
    ("server", "initial-admin-secret", "BOOTSTRAP_ADMIN_KEY"):
        "generated once and kept by lookup, registered at first boot only",
    ("server", "fingerprint-key", "DOCBRAIN_FINGERPRINT_KEY"):
        "generated once and kept by lookup (or the operator's own value); never changes on upgrade",
    ("trainer", "secret", "TRAINER_API_KEY"):
        "optional, and the chart never writes it: only an existingSecret holds it",
    ("trainer", "secret", "azureConnectionString"):
        "optional, and the chart never writes it: only an existingSecret holds it",
}

# The slowest OpenSearch first boot measured on a fresh volume (heavily loaded machine), in seconds.
OPENSEARCH_SLOWEST_FIRST_BOOT_S = 138

EVERYTHING = [
    "--set", "mcpTools.enabled=true",
    "--set", "trainer.enabled=true",
    "--set", "learning.enabled=true",
    "--set-json", 'ingest.sources={"local":{"path":"/data/docs"}}',
    "--set-json", 'freshness.exclusionRules={"archived_title_patterns":["^Archive"]}',
    "--set-json", 'freshness.eventBasedSpaces=["slack_thread"]',
]
VALUE_SETS = {
    "default": [],
    "everything": EVERYTHING,
    "exclusion-rules-only": ["--set-json", 'freshness.exclusionRules={"archived_title_patterns":["^Archive"]}'],
    "event-spaces-only": ["--set-json", 'freshness.eventBasedSpaces=["slack_thread"]'],
    "sources-only": ["--set-json", 'ingest.sources={"local":{"path":"/data/docs"}}'],
    "bootstrap-key-off": ["--set", "bootstrapKey.enabled=false"],
    "existing-secrets": EVERYTHING + ["--set", "existingSecret=my-secret",
                                      "--set", "fingerprintKeySecret.existingSecret=my-fingerprint"],
}
# One content change per object, made on top of EVERYTHING: (object suffix, extra helm args).
CHANGES = [
    ("config", ["--set", "server.env.WIRING_PROBE=changed"]),
    ("secret", ["--set", "llm.anthropicApiKey=wiring-probe"]),
    ("sources", ["--set-json", 'ingest.sources={"local":{"path":"/data/other"}}']),
]

failures = []


def fail(rule, msg):
    failures.append(f"[{rule}] {msg}")


def render(args):
    out = subprocess.run(["helm", "template", RELEASE, CHART, *args], capture_output=True, text=True)
    if out.returncode != 0:
        print(f"CANNOT RUN: helm template {' '.join(args)} failed:\n{out.stderr}", file=sys.stderr)
        sys.exit(2)
    return [d for d in yaml.safe_load_all(out.stdout) if d]


def suffix(name):
    return name[len(FULLNAME) + 1:] if name.startswith(FULLNAME + "-") else None


def pod_template(doc):
    if doc["kind"] == "CronJob":
        return doc["spec"]["jobTemplate"]["spec"]["template"]
    return doc["spec"]["template"]


def component(doc):
    labels = doc["metadata"].get("labels") or {}
    return labels.get("app.kubernetes.io/component") or doc["metadata"]["name"]


def workloads(docs, vs):
    found = {}
    for d in docs:
        spec = d.get("spec") or {}
        has_pod = "template" in spec or "jobTemplate" in spec
        if d["kind"] in POD_KINDS:
            found[component(d)] = d
        elif has_pod:
            fail("reads", f"{vs}: {d['kind']} {d['metadata']['name']} has a pod template but is not a known "
                          f"workload kind; add it to POD_KINDS so every rule covers it")
    return found


def objects(docs):
    """(kind, name) -> set of keys, for every ConfigMap and Secret the chart renders."""
    objs = {}
    for d in docs:
        if d["kind"] in ("ConfigMap", "Secret"):
            keys = set((d.get("data") or {}).keys()) | set((d.get("stringData") or {}).keys())
            objs[(d["kind"], d["metadata"]["name"])] = keys
    return objs


def reads(tmpl):
    """Whole reads [(kind, name)] and key reads [(kind, name, key, optional)] of one pod template."""
    spec = tmpl["spec"]
    whole, keyed = [], []
    for c in spec.get("containers", []) + spec.get("initContainers", []):
        for ef in c.get("envFrom") or []:
            if "configMapRef" in ef:
                whole.append(("ConfigMap", ef["configMapRef"]["name"], ef["configMapRef"].get("optional", False)))
            if "secretRef" in ef:
                whole.append(("Secret", ef["secretRef"]["name"], ef["secretRef"].get("optional", False)))
        for e in c.get("env") or []:
            vf = e.get("valueFrom") or {}
            if "secretKeyRef" in vf:
                r = vf["secretKeyRef"]
                keyed.append(("Secret", r["name"], r["key"], r.get("optional", False), e["name"]))
            if "configMapKeyRef" in vf:
                r = vf["configMapKeyRef"]
                keyed.append(("ConfigMap", r["name"], r["key"], r.get("optional", False), e["name"]))
    for v in spec.get("volumes") or []:
        if "configMap" in v:
            whole.append(("ConfigMap", v["configMap"]["name"], v["configMap"].get("optional", False)))
        if "secret" in v:
            whole.append(("Secret", v["secret"]["secretName"], v["secret"].get("optional", False)))
        for src in (v.get("projected") or {}).get("sources") or []:
            if "configMap" in src:
                whole.append(("ConfigMap", src["configMap"]["name"], src["configMap"].get("optional", False)))
            if "secret" in src:
                whole.append(("Secret", src["secret"]["name"], src["secret"].get("optional", False)))
    return whole, keyed


def check_value_set(vs, docs):
    objs = objects(docs)
    wl = workloads(docs, vs)
    if f"{FULLNAME}-server" not in {d["metadata"]["name"] for d in wl.values()}:
        fail("reads", f"{vs}: no Deployment named {FULLNAME}-server; the test's FULLNAME is out of date")
        return wl
    read_by = {o: [] for o in objs}
    for comp, d in wl.items():
        tmpl = pod_template(d)
        whole, keyed = reads(tmpl)
        expected = set()
        for kind, name, optional in whole:
            if (kind, name) in objs:
                read_by[(kind, name)].append(comp)
                if suffix(name) is None:
                    fail("checksum", f"{vs}: {comp} reads {kind} {name}, which is not named <fullname>-<suffix>")
                else:
                    expected.add(f"checksum/{suffix(name)}")
            elif suffix(name) is not None and not optional:
                fail("reads", f"{vs}: {comp} reads {kind} {name}, which the chart does not render")
        for kind, name, key, optional, env in keyed:
            if (kind, name) in objs:
                read_by[(kind, name)].append(comp)
                if (comp, suffix(name), key) not in KEY_READS:
                    fail("keys", f"{vs}: {comp} reads key {key} of {kind} {name} (env {env}) by reference; list "
                                 f"it in KEY_READS with why a change needs no restart, or read the object whole")
                if key not in objs[(kind, name)] and not optional:
                    fail("reads", f"{vs}: {comp} needs key {key} of {kind} {name}, which renders without it")
            elif suffix(name) is not None and not optional:
                fail("reads", f"{vs}: {comp} needs {kind} {name}, which the chart does not render")
        actual = {k for k in (tmpl["metadata"].get("annotations") or {}) if k.startswith("checksum/")}
        if actual != expected:
            missing, extra = sorted(expected - actual), sorted(actual - expected)
            fail("checksum", f"{vs}: {comp} checksums {sorted(actual)}, but it reads whole {sorted(expected)}"
                             f" (missing {missing}: a change never restarts it; extra {extra}: it restarts for"
                             f" nothing)")
    for (kind, name), comps in read_by.items():
        if not comps:
            fail("reads", f"{vs}: {kind} {name} is rendered but no pod reads it")

    # overlay: every DocBrain pod mounts the chart's local.yaml whenever it renders.
    overlay = ("ConfigMap", f"{FULLNAME}-sources")
    for comp, d in wl.items():
        whole, _ = reads(pod_template(d))
        if ("ConfigMap", f"{FULLNAME}-config") not in {(k, n) for k, n, _ in whole}:
            continue
        mounts = [(m["name"], m["mountPath"], m.get("subPath")) for c in pod_template(d)["spec"]["containers"]
                  for m in c.get("volumeMounts") or []]
        vols = {v["name"]: (v.get("configMap") or {}).get("name") for v in pod_template(d)["spec"].get("volumes") or []}
        mounted = any(p == "/app/config/local.yaml" and sp == "local.yaml" and vols.get(n) == overlay[1]
                      for n, p, sp in mounts)
        if overlay in objs and not mounted:
            fail("overlay", f"{vs}: {comp} runs with the chart's settings but does not mount {overlay[1]}'s "
                            f"local.yaml at /app/config/local.yaml")
        if overlay not in objs and any(p == "/app/config/local.yaml" for _, p, _ in mounts):
            fail("overlay", f"{vs}: {comp} mounts /app/config/local.yaml but the chart renders no overlay")

    # bootstrap: the generated first-login key reaches the server.
    admin = ("Secret", f"{FULLNAME}-initial-admin-secret")
    server = pod_template(wl["server"])
    refs = [r for r in reads(server)[1] if r[4] == "BOOTSTRAP_ADMIN_KEY"]
    if admin in objs:
        ok = [r for r in refs if r[1] == admin[1] and r[2] == "BOOTSTRAP_ADMIN_KEY" and r[3] is True]
        if len(ok) != 1:
            fail("bootstrap", f"{vs}: the chart renders {admin[1]} but the server's BOOTSTRAP_ADMIN_KEY is {refs} "
                              f"(want one optional secretKeyRef to it)")
    elif refs:
        fail("bootstrap", f"{vs}: {admin[1]} is not rendered, yet the server references it: {refs}")
    return wl


def check_startup(vs, wl):
    os_ = wl.get("opensearch")
    if os_ is None:
        return
    for c in pod_template(os_)["spec"]["containers"]:
        sp = c.get("startupProbe")
        if not sp:
            fail("startup", f"{vs}: opensearch container {c['name']} has no startupProbe; its liveness probe "
                            f"restarts a slow first boot")
            continue
        window = sp.get("initialDelaySeconds", 0) + sp.get("periodSeconds", 10) * sp.get("failureThreshold", 3)
        if window < 2 * OPENSEARCH_SLOWEST_FIRST_BOOT_S:
            fail("startup", f"{vs}: opensearch startupProbe allows {window}s, under twice the slowest first boot "
                            f"measured ({OPENSEARCH_SLOWEST_FIRST_BOOT_S}s)")


def pod_digest(d):
    return hashlib.sha256(json.dumps(pod_template(d), sort_keys=True).encode()).hexdigest()


def check_restarts():
    base_docs = render(EVERYTHING)
    base = workloads(base_docs, "everything")
    for obj, extra in CHANGES:
        name = f"{FULLNAME}-{obj}"
        readers = {comp for comp, d in base.items()
                   if any(n == name for _, n, _ in reads(pod_template(d))[0])}
        if not readers:
            fail("restart", f"change of {name}: no pod reads it whole, so the change proves nothing")
            continue
        changed_docs = render(EVERYTHING + extra)
        before = {(d["kind"], d["metadata"]["name"]): d for d in base_docs if d["kind"] in ("ConfigMap", "Secret")}
        after = {(d["kind"], d["metadata"]["name"]): d for d in changed_docs if d["kind"] in ("ConfigMap", "Secret")}
        kind = "Secret" if obj == "secret" else "ConfigMap"
        if before.get((kind, name)) == after.get((kind, name)):
            fail("restart", f"change of {name}: {extra} did not change the object, so the change proves nothing")
            continue
        now = workloads(changed_docs, "changed")
        restarted = {comp for comp in base if comp in now and pod_digest(base[comp]) != pod_digest(now[comp])}
        if restarted != readers:
            fail("restart", f"change of {name}: pods that read it whole {sorted(readers)}, pods whose template "
                            f"changed {sorted(restarted)}")


def main():
    for tool in ("helm",):
        if shutil.which(tool) is None:
            print(f"CANNOT RUN: {tool} is not installed", file=sys.stderr)
            sys.exit(2)
    for vs, args in VALUE_SETS.items():
        wl = check_value_set(vs, render(args))
        check_startup(vs, wl)
    check_restarts()
    if failures:
        print(f"FAIL: {len(failures)} wiring rule(s) broke")
        for f in failures:
            print("  " + f)
        sys.exit(1)
    print(f"PASS: every wiring rule holds over {len(VALUE_SETS)} value sets and {len(CHANGES)} content changes")


if __name__ == "__main__":
    try:
        import yaml
    except ImportError:
        print("CANNOT RUN: PyYAML is not installed (pip install pyyaml)", file=sys.stderr)
        sys.exit(2)
    main()
