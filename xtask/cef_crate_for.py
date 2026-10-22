"""Print the newest `cef` crate version built for a CEF version, e.g.
154.0.34 -> 154.5.0+154.0.34. Exits non-zero when crates.io has none yet, so the
scheduled bump fails loudly (and retries tomorrow) while the crate lags.

usage: cef_crate_for.py CEF_VERSION
"""
import json
import sys
import urllib.request

# Newest first; the crate we want was published days ago, so one page is plenty.
URL = "https://crates.io/api/v1/crates/cef/versions?per_page=100"


def crate_for(cef_version, versions):
    for v in versions:
        if not v["yanked"] and v["num"].partition("+")[2] == cef_version:
            return v["num"]
    return None


def main(argv):
    # crates.io rejects requests without a User-Agent.
    req = urllib.request.Request(URL, headers={"User-Agent": "baa-cef-bump (github.com/dan-hummelstad/baa)"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        versions = json.load(resp)["versions"]
    found = crate_for(argv[1], versions)
    if found is None:
        sys.exit(f"no `cef` crate published for CEF {argv[1]} yet")
    print(found)


if __name__ == "__main__":
    main(sys.argv)
