#!/usr/bin/env python3
"""Candidate VST2 → VST3 identities from FL Studio's own plugin database.

Dev tool, not part of the app. When FL scans plugins it writes one `.nfo`
per plugin under `Presets/Plugin database/Installed/`. Each lists the plugin's
files: a VST3 entry carries the real class ID (`ps_file_guid_N`), a VST2 entry
its unique ID (`ps_file_magic_N`, the FourCC as a big-endian integer). When FL
finds both formats of one plugin it puts them in the same `.nfo`.

    python3 tools/plugindb.py                 # report (Markdown) on stdout
    python3 tools/plugindb.py --tsv           # update assets/known_ids.tsv (merges)
    python3 tools/plugindb.py --db <Installed folder>

What this gives: the VST3 class ID (as FL stores it, COM byte order), the
FourCC, effect or instrument. What it doesn't: how the plugin's state converts
and its parameter-ID table — the `.fst` files hold neither. Each candidate still
needs one conversion checked in FL before it becomes an official rule.
"""

import argparse
import re
import sys
from pathlib import Path

# ps_file_plugclass values seen in FL's database.
VST2_CLASSES = {0: "effect", 4: "instrument"}
VST3_CLASSES = {7: "effect", 8: "instrument"}

# FourCCs that already have a rule in src/plugins.rs.
OFFICIAL = {"XfsX": "Serum", "XfTT": "OTT"}


def default_db() -> Path:
    home = Path.home()
    return home / "Documents" / "Image-Line" / "FL Studio" / "Presets" / "Plugin database" / "Installed"


def read_nfo(path: Path) -> dict:
    fields = {}
    for line in path.read_text(encoding="utf-8-sig", errors="replace").splitlines():
        key, sep, value = line.partition("=")
        if sep:
            fields[key.strip()] = value.strip()
    return fields


def files_of(fields: dict) -> list:
    out = []
    for i in range(int(fields.get("ps_files", "0") or 0)):
        get = lambda k: fields.get(f"ps_file_{k}_{i}")
        cls = get("plugclass")
        out.append(
            {
                "name": get("name") or fields.get("ps_name", ""),
                "path": get("filename") or "",
                "class": int(cls) if cls and cls.lstrip("-").isdigit() else None,
                "guid": get("guid"),
                "magic": int(get("magic")) if (get("magic") or "").isdigit() else None,
                "vendor": get("vendorname") or "",
            }
        )
    return out


def guid_to_com(guid: str) -> bytes:
    """`{D1-D2-D3-D4-D5}` → the 16 bytes FL stores in record 52."""
    h = guid.strip("{}").split("-")
    b = bytes.fromhex("".join(h))
    return b[3::-1] + b[5:3:-1] + b[7:5:-1] + b[8:]


def derived_com(fourcc: int, name: str) -> bytes:
    """Steinberg's rule (mirror of src/vst3.rs)."""
    f = bytearray(16)
    f[:3] = b"VST"
    f[3:7] = fourcc.to_bytes(4, "big")
    low = name.lower().encode("ascii", "replace")[:9]
    f[7 : 7 + len(low)] = low
    f = bytes(f)
    return f[3::-1] + f[5:3:-1] + f[7:5:-1] + f[8:]


def fourcc_str(n: int) -> str:
    b = n.to_bytes(4, "big")
    return b.decode("latin1") if all(32 <= c < 127 for c in b) else f"0x{n:08X}"


def is_vst3(f) -> bool:
    return f["class"] in VST3_CLASSES or f["path"].lower().rstrip("/\\").endswith(".vst3")


def is_vst2(f) -> bool:
    p = f["path"].lower().rstrip("/\\")
    return f["magic"] is not None and (f["class"] in VST2_CLASSES or p.endswith((".vst", ".dll")))


def norm(s: str) -> str:
    return re.sub(r"[^a-z0-9]", "", s.lower())


def collect(db: Path):
    vst2, vst3, paired = {}, {}, {}
    for nfo in sorted(db.rglob("*.nfo")):
        if nfo.name.lower() == "verifiedids.nfo":
            continue
        files = files_of(read_nfo(nfo))
        twos = [f for f in files if is_vst2(f)]
        threes = [f for f in files if is_vst3(f) and f["guid"]]
        for f in twos:
            vst2.setdefault(f["magic"], f)
        for f in threes:
            vst3.setdefault(f["guid"].upper(), f)
        if twos and threes:
            paired.setdefault(twos[0]["magic"], (twos[0], threes[0]))
    return vst2, vst3, paired


def candidates(db: Path):
    vst2, vst3, paired = collect(db)
    out = []
    for magic, two in vst2.items():
        if magic in paired:
            three, source = paired[magic][1], "FL 配对"
        else:
            # Same vendor and name (ignoring a vendor prefix on either side).
            key = norm(two["name"]).removeprefix(norm(two["vendor"]))
            match = [
                t
                for t in vst3.values()
                if norm(t["vendor"]) == norm(two["vendor"])
                and norm(t["name"]).removeprefix(norm(t["vendor"])) == key
            ]
            if len(match) != 1:
                continue
            three, source = match[0], "名字匹配"
        real = guid_to_com(three["guid"])
        out.append(
            {
                "name": two["name"],
                "vendor": two["vendor"] or three["vendor"],
                "fourcc": magic,
                "class_id": real,
                "kind": VST3_CLASSES.get(three["class"], VST2_CLASSES.get(two["class"], "?")),
                "source": source,
                "follows_rule": derived_com(magic, two["name"]) == real,
                "vst3_path": three["path"],
            }
        )
    out.sort(key=lambda c: (c["follows_rule"], c["vendor"].lower(), c["name"].lower()))
    return out


def vst3_only(db: Path, cands) -> list:
    """Installed VST3s without a VST2 pair: still needed to recognise their IDs."""
    _, vst3, _ = collect(db)
    paired = {c["class_id"] for c in cands}
    out = []
    for guid, t in vst3.items():
        cid = guid_to_com(guid)
        if cid in paired:
            continue
        out.append({"name": t["name"], "vendor": t["vendor"], "fourcc": None, "class_id": cid,
                    "kind": VST3_CLASSES.get(t["class"], "?"), "vst3_path": t["path"]})
    return out


def report(cands, db: Path) -> str:
    lines = [
        f"# FL 插件数据库候选（{len(cands)} 个）",
        "",
        f"来源：`{db}`",
        "",
        "推导规则 = Steinberg 的 ID 推导是否给出真实 ID。✗ 的插件现在走通用路径会用错 ID，最该加规则。",
        "",
        "| 插件 | 厂商 | 类型 | VST2 ID | VST3 class ID（FL 存储顺序） | 推导规则 | 配对 | 状态 |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for c in cands:
        fc = fourcc_str(c["fourcc"])
        lines.append(
            f"| {c['name']} | {c['vendor']} | {'效果器' if c['kind'] == 'effect' else '乐器'} | `{fc}` "
            f"| `{c['class_id'].hex()}` | {'✓' if c['follows_rule'] else '✗'} | {c['source']} "
            f"| {'官方支持' if fc in OFFICIAL else ''} |"
        )
    return "\n".join(lines) + "\n"


TSV_HEADER = """\
# VST3 plugins (and their VST2 IDs) from FL Studio's plugin database — read by src/known_ids.rs.
# Generated by `python3 tools/plugindb.py --tsv`; regenerating merges with what's here.
# fourcc: VST2 unique ID, big-endian hex, or - for a VST3 without a VST2.
# class_id: VST3 class ID as FL stores it (record 52).
# The IDs are what each VST3 reported to FL's scanner; state conversion is not verified.
fourcc\tclass_id\tkind\tvst3_file\tvendor\tname
"""


# Rows are keyed by (fourcc, class_id); values are [kind, vst3_file, vendor, name].
def tsv_rows(cands) -> dict:
    rows = {}
    clean = lambda t: re.sub(r"[\t\r\n]", " ", t)
    for c in cands:
        vst3_file = re.split(r"[/\\]", c["vst3_path"].rstrip("/\\"))[-1]
        fourcc = "-" if c["fourcc"] is None else f"{c['fourcc']:08x}"
        rows[(fourcc, c["class_id"].hex())] = [c["kind"], clean(vst3_file), clean(c["vendor"]), clean(c["name"])]
    return rows


def read_tsv(path: Path) -> dict:
    rows = {}
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line and not line.startswith("#") and not line.startswith("fourcc\t"):
                f = line.split("\t")
                rows[(f[0], f[1])] = f[2:]
    return rows


def write_tsv(path: Path, rows: dict):
    # A VST3 listed with its VST2 needs no VST3-only row; one fourcc keeps one row.
    paired = {cid for (fc, cid) in rows if fc != "-"}
    rows = {k: v for k, v in rows.items() if k[0] != "-" or k[1] not in paired}
    lines = sorted(rows.items(), key=lambda kv: (kv[1][2].lower(), kv[1][3].lower(), kv[0]))
    path.write_text(TSV_HEADER + "".join("\t".join([*k, *v]) + "\n" for k, v in lines), encoding="utf-8")


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--db", type=Path, default=default_db(), help="FL's Plugin database/Installed folder")
    ap.add_argument("--tsv", action="store_true", help="update the app's table (assets/known_ids.tsv) instead of printing a report")
    ap.add_argument("--replace", action="store_true", help="with --tsv: drop entries not in this database instead of keeping them")
    args = ap.parse_args()
    if not args.db.is_dir():
        sys.exit(f"找不到 FL 插件数据库：{args.db}（用 --db 指定 Installed 文件夹）")
    cands = candidates(args.db)
    if not args.tsv:
        sys.stdout.write(report(cands, args.db))
        return
    path = Path(__file__).resolve().parent.parent / "assets" / "known_ids.tsv"
    old = {} if args.replace else read_tsv(path)
    new = tsv_rows(cands + vst3_only(args.db, cands))
    # A newer scan's row for a FourCC replaces older ones with another class ID.
    fresh = {fc for (fc, _) in new if fc != "-"}
    rows = {**{k: v for k, v in old.items() if k[0] not in fresh}, **new}
    write_tsv(path, rows)
    changed = sum(1 for k in new if k in old and old[k] != new[k])
    print(f"{path}: {len(rows)} 个插件（本机 {len(new)}，新增 {len(new.keys() - old.keys())}，更新 {changed}，保留其他来源 {len(old.keys() - new.keys())}）")


if __name__ == "__main__":
    main()
