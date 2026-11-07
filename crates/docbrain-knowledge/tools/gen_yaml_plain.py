#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate tests/fixtures/yaml-plain.jsonl: what `scalar::yaml_plain` must return for each plain scalar.

Usage: python3 -I tools/gen_yaml_plain.py > tests/fixtures/yaml-plain.jsonl

The YAML 1.1 reading is PyYAML's own resolver (the deployed 1.1 behaviour), so the fixture
checks the hand-written 1.1 matcher against the real thing. The 1.2 reading is the core schema's regexes as
SEMANTICS.md §Values and comparison states them. Timestamps are text in both. Each line is
{"text": ..., "expect": "Null" | "Bool:true" | "Bool:false" | "Str" | "NotComparable" | "Yaml11" |
"Number:<sign><digits>e<exp>"}, the number canonical (no leading or trailing zeros; zero is "Number:e0").
Needs PyYAML; deterministic for one PyYAML version.
"""

import datetime
import decimal
import itertools
import json
import random
import re
import sys

import yaml

# Exact: no rounding, no overflow, for every number the candidates hold.
decimal.getcontext().prec = 100_000
decimal.getcontext().Emax = decimal.MAX_EMAX
decimal.getcontext().Emin = decimal.MIN_EMIN

R12 = {
    "null": re.compile(r"null|Null|NULL|~|"),
    "bool": re.compile(r"true|True|TRUE|false|False|FALSE"),
    "int10": re.compile(r"[-+]?[0-9]+"),
    "int8": re.compile(r"0o[0-7]+"),
    "int16": re.compile(r"0x[0-9a-fA-F]+"),
    "float": re.compile(r"[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?"),
    "inf": re.compile(r"[-+]?\.(inf|Inf|INF)"),
    "nan": re.compile(r"\.nan|\.NaN|\.NAN"),
}


def canonical(d):
    sign, digits, exp = d.normalize().as_tuple()
    if not any(digits):
        return "Number:e0"
    return "Number:%s%se%d" % ("-" if sign else "", "".join(map(str, digits)), exp)


def read_1_2(t):
    m = lambda k: R12[k].fullmatch(t)
    if m("null"):
        return ("null",)
    if m("bool"):
        return ("bool", t.lower() == "true")
    if m("int8"):
        return ("num", canonical(decimal.Decimal(int(t[2:], 8))))
    if m("int16"):
        return ("num", canonical(decimal.Decimal(int(t[2:], 16))))
    if m("inf"):
        return ("inf", t.startswith("-"))
    if m("nan"):
        return ("nan",)
    if m("int10") or m("float"):
        try:
            return ("num", canonical(decimal.Decimal(t)))
        except decimal.InvalidOperation:
            return None
    return ("str",)


def read_1_1(t):
    """PyYAML's reading, or None when `v: <t>` is not a mapping to one plain scalar with this text."""
    try:
        node = yaml.compose("v: " + t + "\n", Loader=yaml.SafeLoader)
    except yaml.YAMLError:
        return None
    if not isinstance(node, yaml.MappingNode) or len(node.value) != 1:
        return None
    (_, vnode), = node.value
    if not isinstance(vnode, yaml.ScalarNode) or vnode.style is not None or vnode.value != t:
        return None
    try:
        v = yaml.safe_load("v: " + t + "\n")["v"]
    except (yaml.YAMLError, ValueError):
        # The resolver chose a type its constructor cannot build (PyYAML resolves `0x_` as an int): the 1.1
        # reading is that type, which 1.2 never gives.
        return ("unbuildable", vnode.tag)
    if v is None:
        return ("null",)
    if isinstance(v, bool):
        return ("bool", v)
    if isinstance(v, int):
        return ("num", canonical(decimal.Decimal(v)))
    if isinstance(v, float):
        if v != v:
            return ("nan",)
        if v in (float("inf"), float("-inf")):
            return ("inf", v < 0)
        try:
            return ("num", canonical(decimal.Decimal(t.replace("_", ""))))
        except decimal.InvalidOperation:
            return ("float-other", t)  # sexagesimal: 1.2 never reads it as a number
    if isinstance(v, (datetime.date, datetime.datetime, str)):
        return ("str",)
    return None


def expect(t):
    a, b = read_1_2(t), read_1_1(t)
    if a is None or b is None:
        return None
    if t.lower() in ("y", "n", "yes", "no", "on", "off") or a != b:
        return "Yaml11"
    return {
        "null": lambda: "Null",
        "bool": lambda: "Bool:%s" % str(a[1]).lower(),
        "num": lambda: a[1],
        "inf": lambda: "NotComparable",
        "nan": lambda: "NotComparable",
        "str": lambda: "Str",
    }[a[0]]()


def candidates():
    short = "01789+-._:exob"
    for n in range(1, 4):
        for p in itertools.product(short, repeat=n):
            yield "".join(p)
    words = ["y", "n", "yes", "no", "on", "off", "true", "false", "null", "~"]
    for w in words:
        for case in {w, w.upper(), w.capitalize(), w.swapcase(), w[:1] + w[1:].upper()}:
            yield case
    for w in ["inf", "Inf", "INF", "iNf", "nan", "NaN", "NAN", "nAn"]:
        for pre in ["", ".", "+.", "-."]:
            yield pre + w
    yield from ["0x1F", "-0x1F", "+0x1F", "0X1F", "0x1_F", "0o17", "0o8", "0b101", "-0b1_1", "0755", "+0755",
                "010", "08", "00", "07", "0_7", "1_000", "1__0", "1:30", "-1:30", "190:20:30", "190:20:30.15",
                "0:30", "1:60", "1.2.3", "1.5e3", "1.5e+3", "1.e+5", ".5", "+.5", "-.5", "1.", "6.8523015e+5",
                "685.230_15e+03", "2001-12-14", "2001-12-14t21:59:43.10-05:00", "2001-12-14 21:59:43.10 -5",
                "v1.2", "1e5", "1E5", "1e-5", "1.0e5", "12345678901234567890123", "0x" + "f" * 40]
    rng = random.Random(522)
    alphabet = "0123456789" * 3 + "+-._:eEx_ob"
    for _ in range(4000):
        yield "".join(rng.choice(alphabet) for _ in range(rng.randint(4, 10)))


def main():
    seen = set()
    for t in candidates():
        if t in seen:
            continue
        seen.add(t)
        e = expect(t)
        if e is not None:
            print(json.dumps({"text": t, "expect": e}))


if __name__ == "__main__":
    main()
