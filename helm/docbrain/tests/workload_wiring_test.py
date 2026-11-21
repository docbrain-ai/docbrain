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
             A census: every object some pod reads whole has a content change in CHANGES (a value) or
             FILE_CHANGES (a chart file), so a pod whose checksum never moves cannot pass unseen.
  overlay    when the chart's local.yaml renders, every pod that runs DocBrain with the chart's settings (it
             reads <fullname>-config whole) mounts it at /app/config/local.yaml, and the file holds exactly the
             subtrees the values set (sources, freshness.exclusion_rules, freshness.event_based_spaces).
  bootstrap  when the chart generates the first-login admin key, the server is given it. The Secret's name and
             data key are read from the BOOTSTRAP_ADMIN_KEY row of docs/configuration.md, which the server's
             first-boot line is tested against too (docbrain-server, PRESET_BOOTSTRAP_KEY_HINT), so a renamed
             Secret or key fails here until the row changes; the sentence the row says is logged must be the
             one docs/kubernetes.md quotes.
  model      no setting that names a model (a ConfigMap key or container env named *MODEL, *MODEL_ID or
             *MODEL_NAME) renders empty: the reader would take "" as the model instead of the provider's default.
             The "empty-model-ids" and "whitespace-model-ids" value sets blank every model value a template reads
             (found in the templates, so a new one is covered), the second with spaces and a tab, which the chart
             must read as empty. Every such value defaults to empty in values.yaml (the provider's default),
             unless MODEL_DEFAULTS lists it with why; and a value set that sets no model value renders no model
             setting but those defaults, so a provider switched without its model never inherits another
             provider's id.
  startup    every container that runs the OpenSearch image (found by image, so a renamed label cannot hide
             one; none found is a failure) has a startup probe that outlasts twice its slowest measured first
             boot and probes the port its liveness probe probes.
  image      under values that set no image tag, every container (init containers too) runs its image at the
             chart's appVersion, so `helm upgrade` and `helm rollback` move the chart and its images together;
             an image the chart does not build is listed in THIRD_PARTY_IMAGES with why. Every `<x>.image.tag`
             in values.yaml (found there, so a new image is covered) defaults to empty, and the "explicit-tags"
             value set proves a tag set in the values (`latest`) is the tag that runs.
  compose    every Compose file the repository ships (a tracked YAML file whose top level has `services:`, found
             with `git ls-files`, so a new one is covered; none found is a failure) runs every image at the chart's
             appVersion when the environment sets nothing, so `git pull` and `docker compose up -d` move the
             Compose file and its images together, as the chart does. A commented-out `image:` line counts too: an
             operator uncomments it as it is. An image written `${VAR:-default}` is read at its default (a set VAR
             still wins); a variable with no default is a failure. An image the chart does not build is listed in
             THIRD_PARTY_IMAGES, or in COMPOSE_ONLY_IMAGES when the chart does not run it either, with why.

Run: python3 helm/docbrain/tests/workload_wiring_test.py   (from anywhere; renders only, no cluster)
Needs helm, git (the repository must be a git checkout) and PyYAML. A missing tool is a failure (exit 2), never a
pass. Exit 0: every rule held.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

CHART = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
DOCS = os.path.normpath(os.path.join(CHART, "..", "..", "docs"))
REPO = os.path.normpath(os.path.join(CHART, "..", ".."))
RELEASE = "wiring"
FULLNAME = "wiring-docbrain"
POD_KINDS = {"Deployment", "StatefulSet", "DaemonSet", "Job", "CronJob"}

# The values that put a subtree into the chart's local.yaml, and where it lands there.
OVERLAY_VALUES = {
    "ingest.sources": ("sources",),
    "freshness.exclusionRules": ("freshness", "exclusion_rules"),
    "freshness.eventBasedSpaces": ("freshness", "event_based_spaces"),
}

# A value a template reads whose name says it holds a model, and a rendered setting that names one.
MODEL_VALUE = re.compile(r"\.Values\.([A-Za-z0-9_.]*?[mM]odel(?:Id|Name))\b")
MODEL_KEY = re.compile(r"MODEL(_ID|_NAME)?$")

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

# Images the chart runs but does not build, by repository, and why their tag does not follow the chart's appVersion.
THIRD_PARTY_IMAGES = {
    "postgres": "upstream PostgreSQL, pinned by postgresql.image to the major version its data volume was made with",
    "redis": "upstream Redis, pinned by redis.image",
    "opensearchproject/opensearch": "upstream OpenSearch, pinned by opensearch.image to the version its index was "
                                    "made with",
    "busybox": "the OpenSearch pod's sysctl init step; it runs no DocBrain code",
}
EXPLICIT_TAG = "latest"
# Images a Compose file runs that the chart does not run, by repository, and why their tag does not follow the
# chart's appVersion.
COMPOSE_ONLY_IMAGES = {
    "caddy": "upstream Caddy, the single-origin proxy in front of the console and /api; the chart uses an ingress",
}
# `${VAR:-default}` or `${VAR-default}` holding no other variable: Compose runs the default when VAR is not set.
COMPOSE_VARIABLE = re.compile(r"\$\{\w+:?-([^${}]*)\}")
ADMIN_KEY = None  # (Secret name ending, data key, logged sentence), read from docs/configuration.md by main()

# Model values that keep a non-empty default in values.yaml, and why. Every other model value defaults to empty: the
# reader then uses its provider's own default, so switching the provider never keeps another provider's model id.
MODEL_DEFAULTS = {
    "learning.training.baseModelId": "the base model the server and the trainer both fine-tune from; it belongs "
                                     "to no provider, so there is no provider default to fall back to",
}

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
    "bedrock": ["--set", "llm.provider=bedrock", "--set", "embedding.provider=bedrock"],
    "existing-secrets": EVERYTHING + ["--set", "existingSecret=my-secret",
                                      "--set", "fingerprintKeySecret.existingSecret=my-fingerprint"],
}
# One content change per object, made on top of EVERYTHING: (object suffix, extra helm args).
CHANGES = [
    ("config", ["--set", "server.env.WIRING_PROBE=changed"]),
    ("secret", ["--set", "llm.anthropicApiKey=wiring-probe"]),
    ("sources", ["--set-json", 'ingest.sources={"local":{"path":"/data/other"}}']),
]
# One content change per object no value can change, made to a copy of the chart: (object suffix, file under
# the chart, its text). The MCP manifests come from chart files, so a new manifest is the change.
FILE_CHANGES = [
    ("mcp-manifests", "files/mcp-manifests/wiring-probe.yaml", "id: wiring-probe\n"),
]

failures = []


def fail(rule, msg):
    failures.append(f"[{rule}] {msg}")


def render(args, chart=CHART):
    out = subprocess.run(["helm", "template", RELEASE, chart, *args], capture_output=True, text=True)
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
            comp = component(d)
            if comp in found:
                fail("reads", f"{vs}: {d['kind']} {d['metadata']['name']} and {found[comp]['kind']} "
                              f"{found[comp]['metadata']['name']} share the component label {comp!r}; the rules "
                              f"key workloads by it, so one of them would go unchecked")
            found[comp] = d
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


def expected_overlay(args):
    """The chart's local.yaml the values in args should produce, from their --set-json values."""
    want = {}
    for flag, arg in zip(args, args[1:]):
        key, _, raw = arg.partition("=")
        if flag == "--set-json" and key in OVERLAY_VALUES:
            *parents, leaf = OVERLAY_VALUES[key]
            node = want
            for p in parents:
                node = node.setdefault(p, {})
            node[leaf] = json.loads(raw)
    return want


def check_value_set(vs, docs, args):
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
    # ...and holds what the values set, no more and no less.
    want = expected_overlay(args)
    cm = next((d for d in docs if d["kind"] == "ConfigMap" and d["metadata"]["name"] == overlay[1]), None)
    if want:
        got = yaml.safe_load(((cm or {}).get("data") or {}).get("local.yaml") or "") if cm else None
        if got != want:
            fail("overlay", f"{vs}: the chart's local.yaml holds {got!r}, but the values set {want!r}")
    elif cm is not None:
        fail("overlay", f"{vs}: the chart renders {overlay[1]} although none of {sorted(OVERLAY_VALUES)} is set")

    # bootstrap: the generated first-login key reaches the server, in the Secret and key the docs name.
    if ADMIN_KEY is None:
        return wl
    ending, key = ADMIN_KEY[0], ADMIN_KEY[1]
    admin = ("Secret", f"{FULLNAME}{ending}")
    server = pod_template(wl["server"])
    refs = [r for r in reads(server)[1] if r[4] == "BOOTSTRAP_ADMIN_KEY"]
    generates = "bootstrapKey.enabled=false" not in args and not any(a.startswith("existingSecret=") for a in args)
    if generates and admin not in objs:
        fail("bootstrap", f"{vs}: the chart generates the admin key, but renders no {admin[1]}, the Secret "
                          f"docs/configuration.md names (<fullname>{ending})")
    if admin in objs:
        ok = [r for r in refs if r[1] == admin[1] and r[2] == key and r[3] is True]
        if len(ok) != 1:
            fail("bootstrap", f"{vs}: the chart renders {admin[1]} but the server's BOOTSTRAP_ADMIN_KEY is {refs} "
                              f"(want one optional secretKeyRef to its key {key!r}, as docs/configuration.md says)")
        data = next(d for d in docs if (d["kind"], d["metadata"]["name"]) == admin)
        if key not in (data.get("stringData") or {}) and key not in (data.get("data") or {}):
            fail("bootstrap", f"{vs}: {admin[1]} holds no key {key!r}, the one docs/configuration.md names")
    elif refs:
        fail("bootstrap", f"{vs}: {admin[1]} is not rendered, yet the server references it: {refs}")
    return wl


def documented_admin_key():
    """(Secret name ending, data key, logged sentence) from the BOOTSTRAP_ADMIN_KEY row of docs/configuration.md."""
    with open(os.path.join(DOCS, "configuration.md")) as fh:
        rows = [line for line in fh.read().splitlines() if line.startswith("| `BOOTSTRAP_ADMIN_KEY` |")]
    if len(rows) != 1:
        fail("bootstrap", f"docs/configuration.md has {len(rows)} BOOTSTRAP_ADMIN_KEY rows, want one")
        return None
    secret = re.search(r"`<fullname>(-[^`]+)`", rows[0])
    key = re.search(r"\bkey `([^`]+)`", rows[0])
    logged = re.search(r"\blogs `([^`]+)`", rows[0])
    if not (secret and key and logged):
        fail("bootstrap", "the BOOTSTRAP_ADMIN_KEY row of docs/configuration.md must name the Secret "
                          "(`<fullname>-…`), its key (key `…`) and what the server logs (logs `…`): " + rows[0])
        return None
    with open(os.path.join(DOCS, "kubernetes.md")) as fh:
        if f"`{logged.group(1)}`" not in fh.read():
            fail("bootstrap", f"docs/kubernetes.md does not quote `{logged.group(1)}`, the sentence the "
                              f"BOOTSTRAP_ADMIN_KEY row of docs/configuration.md says the server logs")
    return secret.group(1), key.group(1), logged.group(1)


def opensearch_containers(wl):
    """(component, container) for every container running the OpenSearch image, whatever its labels."""
    found = []
    for comp, d in wl.items():
        for c in pod_template(d)["spec"]["containers"]:
            if (c.get("image") or "").split("@")[0].rsplit("/", 1)[-1].split(":")[0] == "opensearch":
                found.append((comp, c))
    return found


def probe_port(probe, container):
    """The port a probe checks, a named port resolved to its number (None for an exec probe)."""
    port = (probe.get("tcpSocket") or probe.get("httpGet") or probe.get("grpc") or {}).get("port")
    if isinstance(port, str):
        port = next((p["containerPort"] for p in container.get("ports") or [] if p.get("name") == port), port)
    return port


def model_values():
    """Every model value (…modelId, …modelName) a template reads, from the template sources."""
    found = set()
    tdir = os.path.join(CHART, "templates")
    for f in sorted(os.listdir(tdir)):
        with open(os.path.join(tdir, f)) as fh:
            found |= set(MODEL_VALUE.findall(fh.read()))
    return sorted(found)


def check_models(vs, docs, args):
    for d in docs:
        if d["kind"] == "ConfigMap":
            for k, v in (d.get("data") or {}).items():
                if MODEL_KEY.search(k) and not str(v).strip():
                    fail("model", f"{vs}: ConfigMap {d['metadata']['name']} sets {k} empty; the reader takes \"\" "
                                  f"as the model instead of its default (render it only when set)")
        if d["kind"] in POD_KINDS:
            spec = pod_template(d)["spec"]
            for c in spec.get("containers", []) + spec.get("initContainers", []):
                for e in c.get("env") or []:
                    if MODEL_KEY.search(e["name"]) and "value" in e and not str(e["value"] or "").strip():
                        fail("model", f"{vs}: {d['metadata']['name']} container {c['name']} sets {e['name']} empty")
    if any(a.startswith(f"{p}=") and a[len(p) + 1:].strip() for a in args for p in model_values()):
        return  # a value set that names a model; one that sets a model value blank or to whitespace sets none
    defaults = model_defaults()
    allowed = {str(defaults[p]) for p in MODEL_DEFAULTS if defaults.get(p) not in ("", None)}
    for d in docs:
        if d["kind"] == "ConfigMap":
            for k, v in (d.get("data") or {}).items():
                if MODEL_KEY.search(k) and str(v) not in allowed:
                    fail("model", f"{vs}: no model value is set, yet ConfigMap {d['metadata']['name']} gives "
                                  f"{k}={v!r} instead of leaving the provider's default")
        if d["kind"] in POD_KINDS:
            spec = pod_template(d)["spec"]
            for c in spec.get("containers", []) + spec.get("initContainers", []):
                for e in c.get("env") or []:
                    if MODEL_KEY.search(e["name"]) and "value" in e and str(e["value"]) not in allowed:
                        fail("model", f"{vs}: no model value is set, yet {d['metadata']['name']} container "
                                      f"{c['name']} gets {e['name']}={e['value']!r}")


def model_defaults():
    """{model value: its values.yaml default} for every model value a template reads."""
    values = chart_file("values.yaml")
    out = {}
    for p in model_values():
        node = values
        for k in p.split("."):
            node = node.get(k) if isinstance(node, dict) else None
        out[p] = node
    return out


def check_model_defaults():
    for p, default in model_defaults().items():
        if p in MODEL_DEFAULTS:
            if default in ("", None):
                fail("model", f"{p} is listed in MODEL_DEFAULTS but defaults to empty; take it off the list")
        elif default not in ("", None):
            fail("model", f"{p} defaults to {default!r} in values.yaml; a model setting defaults to empty (its "
                          f"provider's default), or MODEL_DEFAULTS lists it with why")


def check_startup(vs, wl):
    found = opensearch_containers(wl)
    if not found:
        fail("startup", f"{vs}: no container runs the OpenSearch image, so the rule checked nothing (if a value "
                        f"set turns OpenSearch off, the test must say so)")
    for comp, c in found:
        sp = c.get("startupProbe")
        if not sp:
            fail("startup", f"{vs}: {comp} container {c['name']} runs OpenSearch with no startupProbe; its "
                            f"liveness probe restarts a slow first boot")
            continue
        window = sp.get("initialDelaySeconds", 0) + sp.get("periodSeconds", 10) * sp.get("failureThreshold", 3)
        if window < 2 * OPENSEARCH_SLOWEST_FIRST_BOOT_S:
            fail("startup", f"{vs}: {comp} startupProbe allows {window}s, under twice the slowest first boot "
                            f"measured ({OPENSEARCH_SLOWEST_FIRST_BOOT_S}s)")
        live = c.get("livenessProbe")
        if live and probe_port(sp, c) != probe_port(live, c):
            fail("startup", f"{vs}: {comp} startupProbe checks port {probe_port(sp, c)!r} but its livenessProbe "
                            f"checks {probe_port(live, c)!r}; the startup probe must wait for what the liveness "
                            f"probe will check")


def chart_file(name):
    with open(os.path.join(CHART, name)) as fh:
        return yaml.safe_load(fh)


def image_tag_values():
    """Every `<path>.image.tag` in values.yaml with its default: an image the chart builds is a map with a tag."""
    found = {}

    def walk(node, path):
        if isinstance(node, dict):
            if isinstance(node.get("image"), dict) and "tag" in node["image"]:
                found[".".join(path + ["image", "tag"])] = node["image"]["tag"]
            for k, v in node.items():
                walk(v, path + [k])

    walk(chart_file("values.yaml"), [])
    return found


def split_image(image):
    """(repository, tag) of an image reference; the tag is None when it names none (a digest is not a tag)."""
    name = image.split("@", 1)[0]
    repo, sep, tag = name.rpartition(":")
    if not sep or "/" in tag:  # no tag, or the colon is a registry's port
        return name, None
    return repo, tag


def check_images(vs, wl, args, app_version):
    want = EXPLICIT_TAG if any(re.match(r"[\w.]+\.image\.tag=", a) for a in args) else app_version
    for comp, d in wl.items():
        spec = pod_template(d)["spec"]
        for c in spec.get("containers", []) + spec.get("initContainers", []):
            repo, tag = split_image(c.get("image") or "")
            if repo in THIRD_PARTY_IMAGES:
                continue
            if tag != want:
                fail("image", f"{vs}: {comp} container {c['name']} runs {c.get('image')!r}, not tag {want!r}"
                              + ("" if want == EXPLICIT_TAG else " (the chart's appVersion): the chart and its "
                                 "images would not move together") + "; or list an image the chart does not "
                              "build in THIRD_PARTY_IMAGES")


def compose_files():
    """Every Compose file the repository ships: a tracked YAML file whose top level has `services:`."""
    out = subprocess.run(["git", "-C", REPO, "ls-files", "-z", "--", "*.yml", "*.yaml"],
                         capture_output=True, text=True)
    if out.returncode != 0:
        print(f"CANNOT RUN: git ls-files in {REPO} failed:\n{out.stderr}", file=sys.stderr)
        sys.exit(2)
    found = []
    for rel in sorted(p for p in out.stdout.split("\0") if p):
        path = os.path.join(REPO, rel)
        if os.path.isfile(path):
            with open(path, errors="replace") as fh:
                if re.search(r"^services:", fh.read(), re.M):
                    found.append(rel)
    return found


def compose_default(image):
    """The image Compose runs when the environment sets nothing; None when a variable in it has no default."""
    while True:
        resolved = COMPOSE_VARIABLE.sub(lambda m: m.group(1), image)
        if resolved == image:
            return None if "$" in resolved else resolved
        image = resolved


def check_compose(app_version):
    files = compose_files()
    if not files:
        fail("compose", "no tracked YAML file has a top-level `services:`: the census found no Compose file")
    for rel in files:
        with open(os.path.join(REPO, rel)) as fh:
            text = fh.read()
        try:
            doc = yaml.safe_load(text)
        except yaml.YAMLError as e:
            fail("compose", f"{rel} does not parse: {e}")
            continue
        services = doc.get("services") if isinstance(doc, dict) else None
        if not isinstance(services, dict) or not services:
            fail("compose", f"{rel}: `services:` holds no service, so nothing says which images it runs")
            continue
        images = [(f"service {name}", svc.get("image") if isinstance(svc, dict) else None)
                  for name, svc in services.items()]
        # A commented-out service is one an operator uncomments as it is, so its image follows the same rule.
        images += [(f"the commented-out image on line {text.count(chr(10), 0, m.start()) + 1}", m.group(1))
                   for m in re.finditer(r"^[ \t]*#[ \t]*image:[ \t]*(\S+)", text, re.M)]
        for what, image in images:
            if not isinstance(image, str):
                fail("compose", f"{rel}: {what} names no image, so nothing says which release it runs")
                continue
            default = compose_default(image)
            if default is None:
                fail("compose", f"{rel}: {what} runs {image!r}, which has a variable with no default")
                continue
            repo, tag = split_image(default)
            if repo in THIRD_PARTY_IMAGES or repo in COMPOSE_ONLY_IMAGES:
                continue
            if tag != app_version:
                fail("compose", f"{rel}: {what} runs {default!r} when the environment sets nothing, not tag "
                                f"{app_version!r} (the chart's appVersion): `git pull` and `docker compose up` "
                                f"would not move its image with the file; or list an image the chart does not "
                                f"build in THIRD_PARTY_IMAGES or COMPOSE_ONLY_IMAGES")
    return len(files)


def pod_digest(d):
    return hashlib.sha256(json.dumps(pod_template(d), sort_keys=True).encode()).hexdigest()


def render_with_file(path, text):
    """EVERYTHING rendered from a copy of the chart with one file written (symlinks copied as their targets)."""
    with tempfile.TemporaryDirectory(prefix="wiring-chart-") as tmp:
        copy = os.path.join(tmp, os.path.basename(CHART))
        shutil.copytree(CHART, copy, symlinks=False)
        with open(os.path.join(copy, path), "w") as f:
            f.write(text)
        return render(EVERYTHING, chart=copy)


def check_restarts():
    base_docs = render(EVERYTHING)
    base = workloads(base_docs, "everything")
    objs = objects(base_docs)
    cases = [(f"{FULLNAME}-{o}", " ".join(extra), lambda extra=extra: render(EVERYTHING + extra))
             for o, extra in CHANGES]
    cases += [(f"{FULLNAME}-{o}", f"new chart file {path}", lambda path=path, text=text: render_with_file(path, text))
              for o, path, text in FILE_CHANGES]
    # The census: an object read whole with no change case would never prove its readers restart.
    read_whole = {n for d in base.values() for k, n, _ in reads(pod_template(d))[0] if (k, n) in objs}
    for name in sorted(read_whole - {n for n, _, _ in cases}):
        fail("restart", f"{name} is read whole but has no content change in CHANGES or FILE_CHANGES, so nothing "
                        f"proves a change to it restarts the pods that read it")
    for name, what, render_changed in cases:
        readers = {comp for comp, d in base.items()
                   if any(n == name for _, n, _ in reads(pod_template(d))[0])}
        if not readers:
            fail("restart", f"change of {name}: no pod reads it whole, so the change proves nothing")
            continue
        changed_docs = render_changed()
        before = {(d["kind"], d["metadata"]["name"]): d for d in base_docs if d["kind"] in ("ConfigMap", "Secret")}
        after = {(d["kind"], d["metadata"]["name"]): d for d in changed_docs if d["kind"] in ("ConfigMap", "Secret")}
        kind = next((k for k, n in objs if n == name), None)
        if before.get((kind, name)) == after.get((kind, name)):
            fail("restart", f"change of {name}: {what} did not change the object, so the change proves nothing")
            continue
        now = workloads(changed_docs, "changed")
        restarted = {comp for comp in base if comp in now and pod_digest(base[comp]) != pod_digest(now[comp])}
        if restarted != readers:
            fail("restart", f"change of {name}: pods that read it whole {sorted(readers)}, pods whose template "
                            f"changed {sorted(restarted)}")


def main():
    for tool in ("helm", "git"):
        if shutil.which(tool) is None:
            print(f"CANNOT RUN: {tool} is not installed", file=sys.stderr)
            sys.exit(2)
    global ADMIN_KEY
    ADMIN_KEY = documented_admin_key()
    blanks = model_values()
    if not blanks:
        fail("model", "no template reads a model value, so the census found nothing; MODEL_VALUE is out of date")
    VALUE_SETS["empty-model-ids"] = EVERYTHING + [a for p in blanks for a in ("--set", f"{p}=")]
    VALUE_SETS["whitespace-model-ids"] = EVERYTHING + [a for p in blanks for a in ("--set-string", f"{p}= \t ")]
    check_model_defaults()
    app_version = str(chart_file("Chart.yaml").get("appVersion") or "")
    tags = image_tag_values()
    if not app_version or not tags:
        fail("image", f"appVersion {app_version!r} and image tags {tags!r}: the census found nothing to check")
    for path, default in tags.items():
        if default not in ("", None):
            fail("image", f"{path} defaults to {default!r} in values.yaml; leave it empty, so the image is the "
                          f"chart's appVersion")
    VALUE_SETS["explicit-tags"] = EVERYTHING + [a for p in tags for a in ("--set", f"{p}={EXPLICIT_TAG}")]
    compose_count = check_compose(app_version)
    for vs, args in VALUE_SETS.items():
        docs = render(args)
        wl = check_value_set(vs, docs, args)
        check_startup(vs, wl)
        check_models(vs, docs, args)
        check_images(vs, wl, args, app_version)
    check_restarts()
    if failures:
        print(f"FAIL: {len(failures)} wiring rule(s) broke")
        for f in failures:
            print("  " + f)
        sys.exit(1)
    print(f"PASS: every wiring rule holds over {len(VALUE_SETS)} value sets, "
          f"{len(CHANGES) + len(FILE_CHANGES)} content changes and {compose_count} Compose file(s)")


if __name__ == "__main__":
    try:
        import yaml
    except ImportError:
        print("CANNOT RUN: PyYAML is not installed (pip install pyyaml)", file=sys.stderr)
        sys.exit(2)
    main()
