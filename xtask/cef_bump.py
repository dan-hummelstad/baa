"""Move the workspace to a new `cef` crate version.

usage: cef_bump.py CRATE_VERSION   (e.g. 155.0.0+155.0.3)

Rewrites the `cef = "<major>"` line in the root Cargo.toml and pins both `cef`
and `cef-dll-sys` in Cargo.lock: cargo would otherwise leave cef-dll-sys (which
picks the CEF binary) on whatever still satisfies the new `cef`. Info.plist
needs no edit: `cargo xtask bundle` derives CFBundleVersion from the CEF
version it links.
"""
import pathlib
import re
import subprocess
import sys

PIN = re.compile(r'^cef = "[^"]*"$', re.MULTILINE)


def rewrite_pin(cargo_toml, crate_version):
    major = crate_version.split(".", 1)[0]
    new, n = PIN.subn(f'cef = "{major}"', cargo_toml)
    if n != 1:
        raise SystemExit(f'expected exactly one `cef = "..."` line in Cargo.toml, found {n}')
    return new


def main(crate_version):
    root = pathlib.Path(__file__).resolve().parent.parent
    toml = root / "Cargo.toml"
    toml.write_text(rewrite_pin(toml.read_text(), crate_version))
    for package in ("cef", "cef-dll-sys"):
        subprocess.run(["cargo", "update", "-p", package, "--precise", crate_version], cwd=root, check=True)


if __name__ == "__main__":
    main(sys.argv[1])
