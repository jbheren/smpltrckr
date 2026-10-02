#!/usr/bin/env python3
"""Constitue le corpus de test : des .mod tirés au hasard sur The Mod Archive.

Les modules ne sont pas versionnés (droits variables) : ils vont dans corpus/,
ignoré par git. Les identifiants retenus sont notés dans tests/corpus-ids.txt
pour pouvoir reconstituer le même corpus avec --from-list.

Usage : scripts/fetch-corpus.py [--count 50] [--seed 1] [--from-list]
"""
import argparse, pathlib, random, re, sys, time, urllib.request

URL = "https://api.modarchive.org/downloads.php?moduleid={}"
ROOT = pathlib.Path(__file__).resolve().parent.parent
CORPUS, IDS = ROOT / "corpus", ROOT / "tests" / "corpus-ids.txt"


def fetch(module_id):
    with urllib.request.urlopen(URL.format(module_id), timeout=30) as r:
        name = re.search(r'filename="?([^";]+)', r.headers.get("Content-Disposition", ""))
        return (name.group(1) if name else None), r.read()


def is_mod(name):
    low = name.lower()
    return low.endswith(".mod") or low.startswith("mod.")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=50)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--from-list", action="store_true")
    args = ap.parse_args()
    CORPUS.mkdir(exist_ok=True)

    if args.from_list:
        candidates = [int(l) for l in IDS.read_text().split() if l.strip()]
    else:
        rng = random.Random(args.seed)
        candidates = (rng.randint(1, 210000) for _ in range(args.count * 20))

    kept = []
    for module_id in candidates:
        if len(kept) >= args.count and not args.from_list:
            break
        try:
            name, data = fetch(module_id)
        except Exception as e:
            print(f"{module_id}: {e}", file=sys.stderr)
            continue
        finally:
            time.sleep(1)  # rester poli avec le serveur
        if not name or not is_mod(name) or len(data) < 600:
            continue
        safe = re.sub(r"[^A-Za-z0-9._-]", "_", name)
        (CORPUS / f"{module_id}-{safe}").write_bytes(data)
        kept.append(module_id)
        print(f"{len(kept):3} {module_id} {name}")

    if not args.from_list:
        IDS.parent.mkdir(exist_ok=True)
        IDS.write_text("".join(f"{i}\n" for i in kept))


if __name__ == "__main__":
    main()
