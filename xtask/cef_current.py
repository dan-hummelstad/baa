"""Print the CEF version (X.Y.Z) a Cargo.lock on stdin is locked to.

usage: cef_current.py < Cargo.lock

Cargo.toml only pins the major ("154"); the exact CEF build is the semver
build metadata of `cef-dll-sys` (the crate that downloads the binary), e.g.
154.5.0+154.0.34.
"""
import sys
import tomllib


def locked_cef(lock):
    for package in lock["package"]:
        if package["name"] == "cef-dll-sys":
            return package["version"].split("+", 1)[1]
    raise SystemExit("no `cef-dll-sys` package in Cargo.lock")


if __name__ == "__main__":
    print(locked_cef(tomllib.loads(sys.stdin.read())))
