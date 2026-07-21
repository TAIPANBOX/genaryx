#!/usr/bin/env python3
"""Record WHEN each screenshot was taken and WHAT the totals were at that moment.

Why this exists
---------------
The previous campaign shipped screenshots that disagreed with each other, and
the tempting fix was a caption: "captured during different tests, numbers may
not match". That is an apology, not evidence. For a FinOps and banking
audience an apology about inconsistent figures costs more credibility than the
inconsistency itself.

This does the opposite. It turns "trust us" into "check us": every shot is
pinned to an EPOCH, and an epoch is a stretch of time during which the dataset
was frozen (no seeding, no trickle, no kills). Shots inside one epoch are
consistent BY CONSTRUCTION, not by luck, because the numbers behind them could
not move. Where two shots genuinely belong to different epochs, the manifest
says so plainly, with both sets of totals, so a reader can see exactly what
changed and why rather than being asked to overlook it.

How it is used
--------------
    # freeze the data, then open an epoch and describe it
    python3 gx_manifest.py open  --label "console tabs, trickle stopped"

    ...take screenshots...

    # close it, then write the manifest over every shot recorded so far
    python3 gx_manifest.py close
    python3 gx_manifest.py write

`open` and `close` read the live totals from the Cloud themselves, so the
numbers in the manifest are observed, never typed in by hand. If a value moved
between `open` and `close`, the epoch was NOT frozen and the manifest marks it
DRIFTED instead of quietly averaging the two.
"""
import json
import os
import pathlib
import sys
import time
import urllib.request

CLOUD = "http://127.0.0.1:8080"
BEARER = "devkey"
SHOTS = pathlib.Path(__file__).resolve().parent.parent / "shots" / "2026-07-20"
EPOCHS = SHOTS / "epochs.jsonl"


def cloud(path):
    req = urllib.request.Request(f"{CLOUD}{path}",
                                 headers={"Authorization": f"Bearer {BEARER}"})
    return json.loads(urllib.request.urlopen(req, timeout=15).read())


def totals():
    """The three numbers every surface ultimately renders, read at this instant."""
    s = cloud("/v1/summary")
    return {
        "runs": s["runs"],
        "calls": s["calls"],
        "spent_usd": round(s["spent_microusd"] / 1e6, 2),
        "killed": len(cloud("/v1/kills")),
    }


def load_epochs():
    if not EPOCHS.exists():
        return []
    return [json.loads(l) for l in EPOCHS.read_text().splitlines() if l.strip()]


def save_epochs(rows):
    EPOCHS.parent.mkdir(parents=True, exist_ok=True)
    EPOCHS.write_text("".join(json.dumps(r) + "\n" for r in rows))


def cmd_open(label):
    rows = load_epochs()
    if rows and rows[-1].get("closed_unix") is None:
        sys.exit(f"epoch {rows[-1]['id']} is still open; close it first")
    # Derive the next id from the highest id in use, NOT from the row count.
    # Counting rows collides the moment a row is removed or one is written by
    # hand, and it did: two epochs both came out as `e02`, which makes the shot
    # table ambiguous about which totals a given file belongs to.
    used = [int(r["id"][1:]) for r in rows if r.get("id", "").startswith("e")
            and r["id"][1:].isdigit()]
    rows.append({
        "id": f"e{(max(used) + 1) if used else 1:02d}",
        "label": label,
        "opened_unix": int(time.time()),
        "opened_totals": totals(),
        "closed_unix": None,
        "closed_totals": None,
    })
    save_epochs(rows)
    print(f"opened {rows[-1]['id']}: {label}")
    print(f"  {rows[-1]['opened_totals']}")


def cmd_close():
    rows = load_epochs()
    if not rows or rows[-1].get("closed_unix") is not None:
        sys.exit("no open epoch")
    rows[-1]["closed_unix"] = int(time.time())
    rows[-1]["closed_totals"] = totals()
    save_epochs(rows)
    o, c = rows[-1]["opened_totals"], rows[-1]["closed_totals"]
    print(f"closed {rows[-1]['id']}")
    print(f"  frozen: {o == c}")
    if o != c:
        for k in o:
            if o[k] != c[k]:
                print(f"    {k}: {o[k]} -> {c[k]}")


def epoch_of(mtime, rows):
    for r in rows:
        end = r["closed_unix"] or (r["opened_unix"] + 86400)
        if r["opened_unix"] - 5 <= mtime <= end + 5:
            return r
    return None


def cmd_write():
    rows = load_epochs()
    shots = sorted(p for p in SHOTS.rglob("*.png"))
    if not shots:
        sys.exit("no screenshots found")

    entries = []
    for p in shots:
        m = int(p.stat().st_mtime)
        e = epoch_of(m, rows)
        entries.append({
            "file": str(p.relative_to(SHOTS)),
            "captured_local": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(m)),
            "epoch": e["id"] if e else None,
            "epoch_label": e["label"] if e else "outside any recorded epoch",
            "totals": (e["opened_totals"] if e else None),
        })

    (SHOTS / "manifest.json").write_text(json.dumps({
        "dataset": "meridian.example, seeded by gx_fleet_v3.py",
        "box": "5.75.234.176 (Hetzner CPX62)",
        "generated_local": time.strftime("%Y-%m-%d %H:%M:%S"),
        "epochs": rows,
        "shots": entries,
    }, indent=2) + "\n")

    md = ["# Screenshot manifest - 2026-07-20", "",
          "Every shot below is pinned to an **epoch**: a stretch of time during which",
          "the dataset was frozen. Shots sharing an epoch are consistent because the",
          "numbers behind them could not move, not because they were checked afterwards.",
          "", "## Epochs", "",
          "| Epoch | What was being captured | Frozen | Runs | Calls | Spend | Killed |",
          "|---|---|---|---|---|---|---|"]
    for r in rows:
        t = r["opened_totals"]
        frozen = "yes" if r.get("closed_totals") == t else (
            "NO, DRIFTED" if r.get("closed_totals") else "still open")
        md.append(f"| `{r['id']}` | {r['label']} | {frozen} | {t['runs']:,} | "
                  f"{t['calls']:,} | ${t['spent_usd']:,.2f} | {t['killed']} |")

    md += ["", "## Shots", "",
           "A shot with no epoch was taken while the dataset was deliberately",
           "moving (the fleet was still being trickled) or belongs to an earlier",
           "run of the campaign. That is stated rather than smoothed over: those",
           "files show a real moment, but their totals are not comparable with",
           "the frozen set above and must not be quoted as if they were.",
           "",
           "| File | Captured | Epoch |", "|---|---|---|"]
    for e in entries:
        md.append(f"| `{e['file']}` | {e['captured_local']} | "
                  f"`{e['epoch'] or 'none'}` |")
    md += ["", "Regenerate with `python3 scripts/gx_manifest.py write`.", ""]
    (SHOTS / "MANIFEST.md").write_text("\n".join(md))
    print(f"wrote MANIFEST.md and manifest.json over {len(entries)} shot(s), "
          f"{len(rows)} epoch(s)")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    if sys.argv[1] == "open":
        cmd_open(sys.argv[3] if len(sys.argv) > 3 else "unlabelled")
    elif sys.argv[1] == "close":
        cmd_close()
    elif sys.argv[1] == "write":
        cmd_write()
    else:
        sys.exit(f"unknown command {sys.argv[1]}")
