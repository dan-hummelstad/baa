"""Print the newest stable CEF version (X.Y.Z) that every given platform has.

usage: cef_latest.py index.json PLATFORM...
"""
import json
import re
import sys

# Pre-2019 entries use "3.3626.1895.g7001d56"; only the modern form is comparable.
MODERN = re.compile(r"^(\d+\.\d+\.\d+)\+")


def latest_common_stable(index, platforms):
    per_platform = [
        {
            m.group(1)
            for v in index[p]["versions"]
            if v["channel"] == "stable" and (m := MODERN.match(v["cef_version"]))
        }
        for p in platforms
    ]
    common = set.intersection(*per_platform)
    if not common:
        return None
    return max(common, key=lambda v: tuple(map(int, v.split("."))))


def main(argv):
    with open(argv[1]) as f:
        index = json.load(f)
    found = latest_common_stable(index, argv[2:])
    if found is None:
        sys.exit(f"no stable CEF version shared by {', '.join(argv[2:])}")
    print(found)


if __name__ == "__main__":
    main(sys.argv)
