#!/usr/bin/env python3
"""A crate's public API as a page: every item a program can name, with its
signature and the first sentence of its documentation, from rustdoc's own
list (its JSON output), so the page names exactly what the crate offers.

    python3 tools/api.py DOC.json                 # the list
    python3 tools/api.py DOC.json --check PAGE    # PAGE's list is this one
    python3 tools/api.py DOC.json --check         # the two checks below alone

`--check` also holds two inventories to the crate (BLAKE3-Servil 0.9's
wrong hash, GHSA-v237-j9h9-8pf7, was a trait implementation that no
sentence described and no test walked together with the call it broke):

- Every public type that implements a trait (derived or written; auto
  traits and blanket implementations aside) has in its documentation a
  section "# Trait implementations" with one line for each: the trait's
  name in backquotes, then a sentence that says what the implementation
  gives and what it keeps true. A trait with no line, or a line with no
  trait, fails.
- Every operation (public method or trait implementation) of a type in
  LIFECYCLES is named in its lifecycle test, in the comment block
  "// WALKED", or in "// NOT_WALKED" with a reason. The core's walk also
  names the free functions whose signature mentions Hasher. An operation
  in neither list, or a name that is no operation, fails.

To cover another type, add its rustdoc name to LIFECYCLES for its crate
and name its operations in that test. The checks compare names: whether
a sentence is true and a walk tests what it names is for review.

A page is its own opening, then the list from the line "## Crate root"
to its end; `--check` compares that part and prints what differs
(tools/check-builds.sh runs it on docs/core-api.md).

DOC.json: `RUSTC_BOOTSTRAP=1 cargo rustdoc --lib --all-features --
-Z unstable-options --output-format json` writes it under target/doc/
(every feature: an implementation behind one is the kind that was missed).
Hidden items (`#[doc(hidden)]`) are left out: they are not for programs.
"""
import json
import re
import sys
from pathlib import Path

LIFECYCLES = {
    "blake3_servil_core": ("tests/lifecycle.rs", {"Hasher", "OutputReader"}),
    "blake3_servil": ("threads/tests/lifecycle.rs", {"Pipeline", "Room", "Message", "Lent"}),
}


def compare_names(label, expected, actual):
    missing, stale = sorted(expected - actual), sorted(actual - expected)
    if missing or stale:
        raise ValueError(f"{label}: missing {missing}; stale {stale}")


def check_traits(name, traits, docs):
    sections = re.split(r"^# Trait implementations[ \t]*$", docs or "", flags=re.M)
    if not traits and len(sections) == 1:
        return
    if len(sections) != 2:
        raise ValueError(f"{name}: expected one # Trait implementations section for {sorted(traits)}")
    body = re.split(r"^# \S", sections[1], maxsplit=1, flags=re.M)[0]
    # A line may run on: what follows a "- " up to a blank line is one item.
    items = []
    for line in map(str.strip, body.splitlines()):
        if line.startswith(("- ", "* ")):
            items.append(line[2:])
        elif line and items and items[-1] is not None:
            items[-1] += " " + line
        else:
            items.append(None)
    names = set()
    for item in filter(None, items):
        m = re.fullmatch(r"`(\w+)` (\S.*\.)", item)
        if not m or m[1] in names:
            raise ValueError(f"{name}: malformed or duplicate trait line: {item}")
        names.add(m[1])
    compare_names(name + " trait contracts", traits, names)


def check_walk(label, operations, text):
    blocks = re.findall(r"^// WALKED\n((?:// [^\n]*\n)*?)// NOT_WALKED\n((?:// [^\n]*\n)*?)// END$", text, re.M)
    if len(blocks) != 1:
        raise ValueError(f"{label}: expected WALKED, NOT_WALKED and END")
    walked, omitted = blocks[0]
    names = [word for line in walked.splitlines() for word in line[3:].split()]
    for line in omitted.splitlines():
        m = re.fullmatch(r"// (\w+::\w+): (\S.*)", line)
        if not m:
            raise ValueError(f"{label}: expected an operation and reason in {line}")
        names.append(m[1])
    if len(names) != len(set(names)):
        raise ValueError(f"{label}: duplicate walk declaration")
    compare_names(label, operations, set(names))


def ty(t):
    if t is None:
        return "()"
    k, v = next(iter(t.items()))
    if k == "resolved_path":
        s = (v.get("path") or v.get("name")).split("::")[-1]
        a = v.get("args")
        if a and "angle_bracketed" in a:
            xs = []
            for g in a["angle_bracketed"]["args"]:
                if "type" in g:
                    xs.append(ty(g["type"]))
                elif "lifetime" in g:
                    xs.append(g["lifetime"])
                elif "const" in g:
                    xs.append(str(g["const"].get("expr", "_")))
            if xs:
                s += "<" + ", ".join(xs) + ">"
        return s
    if k in ("primitive", "generic"):
        return v
    if k == "borrowed_ref":
        return "&" + ((v["lifetime"] + " ") if v.get("lifetime") else "") + ("mut " if v.get("is_mutable") else "") + ty(v["type"])
    if k == "slice":
        return "[" + ty(v) + "]"
    if k == "array":
        return "[" + ty(v["type"]) + "; " + str(v["len"]) + "]"
    if k == "tuple":
        return "(" + ", ".join(ty(x) for x in v) + ")"
    if k == "impl_trait":
        return "impl " + " + ".join(bound(b) for b in v)
    if k == "dyn_trait":
        bounds = [trait_ty(p["trait"]) for p in v["traits"]]
        if v.get("lifetime"):
            bounds.append(v["lifetime"])
        s = "dyn " + " + ".join(bounds)
        return "(" + s + ")" if len(bounds) > 1 else s
    if k == "qualified_path":
        return ty(v["self_type"]) + "::" + v["name"]
    return k


def trait_ty(t):
    s = t["path"].split("::")[-1]
    a = t.get("args")
    if a and "angle_bracketed" in a:
        xs = [ty(g["type"]) for g in a["angle_bracketed"]["args"] if "type" in g]
        xs += [c["name"] + " = " + ty(c["binding"]["equality"]["type"]) for c in a["angle_bracketed"].get("constraints", []) if "type" in c.get("binding", {}).get("equality", {})]
        if xs:
            s += "<" + ", ".join(xs) + ">"
    if a and "parenthesized" in a:
        s += "(" + ", ".join(ty(x) for x in a["parenthesized"]["inputs"]) + ")"
        if a["parenthesized"].get("output"):
            s += " -> " + ty(a["parenthesized"]["output"])
    return s


def bound(b):
    if "trait_bound" in b:
        tb = b["trait_bound"]
        prefix = "?" if tb.get("modifier") == "maybe" else ""
        return prefix + trait_ty(tb["trait"])
    return b.get("outlives", "?")


def generics(g):
    xs = []
    for p in g.get("params", []):
        k = p["kind"]
        if "lifetime" in k:
            xs.append(p["name"])
        elif "type" in k and not k["type"].get("is_synthetic"):
            bs = " + ".join(bound(b) for b in k["type"].get("bounds", []))
            xs.append(p["name"] + (": " + bs if bs else ""))
        elif "const" in k:
            xs.append("const " + p["name"] + ": " + ty(k["const"]["type"]))
    return "<" + ", ".join(xs) + ">" if xs else ""


def fn(it):
    f = it["inner"]["function"]
    s, h = f["sig"], f.get("header", {})
    args = []
    for n, t in s["inputs"]:
        if n == "self":
            args.append({"Self": "self", "&Self": "&self", "&mut Self": "&mut self"}.get(ty(t), "self: " + ty(t)))
        else:
            args.append(n + ": " + ty(t))
    pre = ("const " if h.get("is_const") else "") + ("unsafe " if h.get("is_unsafe") else "")
    return pre + "fn " + it["name"] + generics(f.get("generics", {})) + "(" + ", ".join(args) + ")" + (" -> " + ty(s["output"]) if s.get("output") else "")


def hidden(it):
    return any("hidden" in str(a) for a in it.get("attrs", []))


def first_sentence(it):
    d = (it.get("docs") or "").strip()
    if not d:
        return ""
    para = " ".join(d.split("\n\n")[0].split())
    para = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", para)  # [text](link) -> text
    para = re.sub(r"\[(`[^`]+`)\]", r"\1", para)            # [`item`] -> `item`
    m = re.match(r"(.+?[.:])(\s|$)", para)
    return (m.group(1) if m else para).rstrip(":") .rstrip(".") + "."


out = []
seen = set()


def show_type(it, path):
    kind = next(iter(it["inner"]))
    inner = it["inner"][kind]
    g = generics(inner.get("generics", {})) if isinstance(inner, dict) else ""
    out.append(f"\n### `{kind} {path.split('::')[-1]}{g}`\n")
    if first_sentence(it):
        out.append(first_sentence(it) + "\n")
    if it["id"] in seen:
        return
    seen.add(it["id"])
    lines, traits, methods = [], [], []
    if kind == "enum":
        for vid in inner["variants"]:
            v = ix[str(vid)]
            vk = v["inner"]["variant"]["kind"]
            if isinstance(vk, dict) and "tuple" in vk:
                lines.append(v["name"] + "(" + ", ".join(ty(ix[str(f)]["inner"]["struct_field"]) for f in vk["tuple"] if f is not None) + ")")
            else:
                lines.append(v["name"])
    for iid in inner.get("impls", []) if isinstance(inner, dict) else []:
        i = ix[str(iid)]["inner"]["impl"]
        if i.get("is_synthetic") or i.get("blanket_impl"):
            continue
        # rustdoc also lists, under a type, the implementations that only
        # mention it (`From<Cv> for LastCv` under `Cv`): another type's.
        if i.get("trait") and i["for"].get("resolved_path", {}).get("id") != it["id"]:
            continue
        if i.get("trait"):
            trait = i["trait"]["path"].split("::")[-1]
            if trait != "StructuralPartialEq":
                traits.append(trait)
        else:
            for mid in i["items"]:
                m = ix[str(mid)]
                if m["visibility"] != "public" or hidden(m):
                    continue
                mk = next(iter(m["inner"]))
                if mk == "function":
                    lines.append(fn(m))
                    methods.append(m["name"])
                elif mk in ("assoc_const", "constant"):
                    lines.append("const " + m["name"])
    if checking:
        name = it["name"]
        check_traits(name, set(traits), it.get("docs"))
        if name in covered:
            operations.update(f"{name}::{op}" for op in methods + traits)
    if lines:
        out.append("```rust\n" + "\n".join(lines) + "\n```\n")
    if traits:
        out.append("Implements: " + ", ".join(f"`{t}`" for t in sorted(set(traits))) + ".\n")


def walk(mid, path, depth):
    m = ix[str(mid)]
    fns, types, consts, mods = [], [], [], []
    for cid in m["inner"]["module"]["items"]:
        it = ix[str(cid)]
        if it["visibility"] != "public" or hidden(it):
            continue
        kind = next(iter(it["inner"]))
        if kind == "use":
            u = it["inner"]["use"]
            tid = u.get("id")
            if tid is None or str(tid) not in ix:
                consts.append((f"{u['name']}", f"(from `{u['source'].split('::')[0]}`)"))
                continue
            target = ix[str(tid)]
            if hidden(target):
                continue
            tk = next(iter(target["inner"]))
            (fns if tk == "function" else types if tk in ("struct", "enum", "union", "trait", "type_alias") else mods if tk == "module" else consts).append((u["name"], target) if tk != "function" else target)
        elif kind == "module":
            mods.append((it["name"], it))
        elif kind == "function":
            fns.append(it)
        elif kind in ("struct", "enum", "union", "trait", "type_alias"):
            types.append((it["name"], it))
        elif kind == "constant":
            consts.append((it["name"], it))
    if consts:
        out.append("\n```rust")
        for name, it in consts:
            out.append(f"const {name}" + (f": {ty(it['inner']['constant']['type'])}" if isinstance(it, dict) else f"  {it}"))
        out.append("```\n")
    if fns:
        out.append("")
        for f in fns:
            if checking and "Hasher" in covered and re.search(r"\bHasher\b", fn(f)):
                operations.add((path + "::" + f["name"]).split("::", 1)[1])
            out.append(f"- `{fn(f)}`  \n  {first_sentence(f)}")
        out.append("")
    for name, it in types:
        show_type(it, path + "::" + name)
    for name, it in mods:
        out.append(f"\n{'#' * depth} Module `{path.split('::', 1)[-1] + '::' if '::' in path else ''}{name}`\n")
        if first_sentence(it):
            out.append(first_sentence(it) + "\n")
        walk(it["id"], path + "::" + name, depth)


def main():
    global ix, covered, checking, operations
    doc = json.loads(Path(sys.argv[1]).read_text())
    ix = doc["index"]
    crate = ix[str(doc["root"])]["name"]
    checking = sys.argv[2:3] == ["--check"]
    if checking:
        test, covered = LIFECYCLES[crate]
        operations = set()
    walk(doc["root"], crate, 2)
    listed = "\n".join(out).strip() + "\n"
    if not checking:
        print(listed, end="")
        return
    check_walk(test, operations, (Path(__file__).resolve().parents[1] / test).read_text())
    if len(sys.argv) == 3:
        return
    import difflib

    page = open(sys.argv[3]).read()
    start = "## Crate root\n\n"
    assert start in page, f'{sys.argv[3]}: no line "## Crate root" to start its list'
    shown = page[page.index(start) + len(start):]
    if shown != listed:
        sys.stdout.writelines(difflib.unified_diff(shown.splitlines(True), listed.splitlines(True), sys.argv[3], "the crate"))
        sys.exit(f"{sys.argv[3]}: its list differs from the crate's; write the list again (python3 tools/api.py DOC.json)")


if __name__ == "__main__":
    try:
        main()
    except ValueError as error:
        sys.exit(str(error))
