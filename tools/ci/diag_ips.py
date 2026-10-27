#!/usr/bin/env python3
# Diagnostic only (claude/diag-macos-segv): condense macOS .ips crash
# reports into the exception, the faulting thread's frames and every
# thread's top frames.
import json
import sys


def frames(report, thread, limit):
    images = report.get("usedImages", [])
    out = []
    for f in thread.get("frames", [])[:limit]:
        idx = f.get("imageIndex")
        img = images[idx] if idx is not None and idx < len(images) else {}
        name = img.get("name") or img.get("path", "?").rsplit("/", 1)[-1]
        sym = f.get("symbol", "?")
        loc = f.get("symbolLocation", 0)
        src = ""
        if "sourceFile" in f:
            src = f"  ({f['sourceFile']}:{f.get('sourceLine', '?')})"
        out.append(f"    {name:28} {sym} + {loc}{src}  [off {f.get('imageOffset')}]")
    return out


for path in sys.argv[1:]:
    print(f"===== {path}")
    text = open(path, encoding="utf-8", errors="replace").read()
    first, _, rest = text.partition("\n")
    try:
        report = json.loads(rest)
    except Exception as e:  # noqa: BLE001
        print(f"(not JSON: {e})")
        print(text[:20000])
        continue
    print(first)
    for key in ("exception", "termination", "vmRegionInfo", "asi", "ktriageinfo"):
        if key in report:
            print(f"{key}: {json.dumps(report[key])[:2000]}")
    ft = report.get("faultingThread")
    threads = report.get("threads", [])
    if ft is not None and ft < len(threads):
        t = threads[ft]
        print(f"--- faulting thread {ft} {t.get('name', '')} {t.get('queue', '')}")
        if "threadState" in t:
            ts = t["threadState"]
            regs = {k: ts[k] for k in ("pc", "lr", "sp", "far", "esr") if k in ts}
            print(f"    state: {json.dumps(regs)[:1000]}")
        print("\n".join(frames(report, t, 80)))
    for i, t in enumerate(threads):
        if i == ft:
            continue
        print(f"--- thread {i} {t.get('name', '')} {t.get('queue', '')}")
        print("\n".join(frames(report, t, 30)))
    imgs = report.get("usedImages", [])
    print("--- images (vulkan/moltenvk/sdl3)")
    for img in imgs:
        n = (img.get("name") or img.get("path") or "").lower()
        if "vulkan" in n or "sdl3" in n or "metal" in n:
            print(f"    {img.get('base')} {img.get('size')} {img.get('path')}")
