#!/usr/bin/env python3
"""Runs the BaaCore harness tests (tests/*.expect) against a bundled harness app.

Called by ci.sh, which builds and bundles the harness. Each test:
  - runs the harness (src/main.rs) with a script (tests/<script>), a fresh --user-data-dir and the
    env auto-answers its .expect file asks for, until the script quits;
  - is checked against the harness log: patterns that must / must not appear, counts, the exit
    code, crash dumps, and CDP probes (cdp.mjs) taken while it runs.

.expect syntax, one `key: value` per line, `#` comments:
  script: <file in tests/>          required
  env: NAME=value                   harness env (BAA_HARNESS_*, BAA_SPIKE_*)
  profile: Default                  page profile (BAA_SPIKE_PROFILE); default space-1
  ext: yes                          load test-ext (into the Default profile)
  server: yes                       start tests/download-server.py; {SERVER} in the script
  secs: <n>                         time limit (default 90)
  exit: any                         the app may not exit by itself (killed after kill-after)
  kill-after: <n>                   with `exit: any`, kill the app after n seconds
  cdp-after: <n>                    seconds after start when the cdp: probes run (Browser.close follows)
  cdp-settle: <n>                   seconds between the last probe and Browser.close (default 1)
  cdp: <target filter> :: <js> ==> <regex>      a probe; the result JSON must match
  must: <regex>                     found in the log (multi-line mode)
  mustnot: <regex>
  count: <n> <regex>                exactly n matching lines
  clipboard-set: <text>             before the run
  clipboard: <text>                 the pasteboard afterwards
  exists: <path>                    under the user data dir afterwards
{EXT_ID} in a script is the test extension's id.
"""

import argparse
import glob
import hashlib
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.realpath(__file__))
TESTS = os.path.join(HERE, "tests")
# Whatever a test expects, these are never fine.
ALWAYS_NOT = r"FATAL|Check failed|panicked|Abort trap|NOTREACHED"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def ext_id(path):
    """Chromium's id for an unpacked extension: sha256 of its path, nibbles mapped to a-p."""
    digest = hashlib.sha256(path.encode()).digest()[:16]
    return "".join(chr(97 + (b >> 4)) + chr(97 + (b & 15)) for b in digest)


class Spec:
    def __init__(self, path):
        self.name = os.path.basename(path)[: -len(".expect")]
        self.script = None
        self.env, self.profile = {}, None
        self.ext = self.server = False
        self.secs, self.exit_any, self.kill_after = 90, False, None
        self.cdp_after, self.cdp_settle, self.cdp = None, 1, []
        self.must, self.mustnot, self.count, self.exists = [], [], [], []
        self.clipboard_set = self.clipboard = None
        for n, raw in enumerate(open(path, encoding="utf-8"), 1):
            line = raw.rstrip("\n")
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            key, _, value = line.partition(":")
            value = value.strip()
            if key == "script":
                self.script = value
            elif key == "env":
                k, _, v = value.partition("=")
                self.env[k] = v
            elif key == "profile":
                self.profile = value
            elif key == "ext":
                self.ext = value == "yes"
            elif key == "server":
                self.server = value == "yes"
            elif key == "secs":
                self.secs = int(value)
            elif key == "exit":
                self.exit_any = value == "any"
            elif key == "kill-after":
                self.kill_after = int(value)
            elif key == "cdp-after":
                self.cdp_after = int(value)
            elif key == "cdp-settle":
                self.cdp_settle = int(value)
            elif key == "cdp":
                target, _, rest = value.partition(" :: ")
                expr, _, regex = rest.partition(" ==> ")
                self.cdp.append((target.strip(), expr.strip(), regex.strip()))
            elif key == "must":
                self.must.append(value)
            elif key == "mustnot":
                self.mustnot.append(value)
            elif key == "count":
                n_, _, regex = value.partition(" ")
                self.count.append((int(n_), regex))
            elif key == "clipboard-set":
                self.clipboard_set = value
            elif key == "clipboard":
                self.clipboard = value
            elif key == "exists":
                self.exists.append(value)
            else:
                sys.exit(f"{path}:{n}: unknown key {key!r}")
        if not self.script:
            sys.exit(f"{path}: no script:")


# The files the tests download, by exact name; nothing else in ~/Downloads is ever touched.
TEST_DOWNLOADS = ("baa-small.bin", "baa-slow.bin", "baa-download.txt")


def clean_downloads():
    for name in TEST_DOWNLOADS:
        try:
            os.remove(os.path.join(os.path.expanduser("~/Downloads"), name))
        except OSError:
            pass


def run_cdp(port, target, expr):
    cmd = ["node", os.path.join(HERE, "cdp.mjs"), str(port), expr] + ([target] if target else [])
    out = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
    return (out.stdout + out.stderr).strip()


def run_test(spec, exe, keep):
    # RUNNER_TEMP on CI, where a failed run's logs are uploaded from.
    tmp = tempfile.mkdtemp(prefix=f"baa-ci-{spec.name}-", dir=os.environ.get("RUNNER_TEMP"))
    udd = os.path.join(tmp, "profile")
    log_path = os.path.join(tmp, "log")
    problems = []
    ext_path = os.path.join(HERE, "test-ext")
    server = None
    port = None
    proc = None
    try:
        text = open(os.path.join(TESTS, spec.script), encoding="utf-8").read()
        text = text.replace("{EXT_ID}", ext_id(os.path.realpath(ext_path)))
        if spec.server:
            sp = free_port()
            server = subprocess.Popen([sys.executable, os.path.join(TESTS, "download-server.py"), str(sp)])
            text = text.replace("{SERVER}", f"http://127.0.0.1:{sp}")
            time.sleep(0.5)
        script = os.path.join(tmp, "script.txt")
        open(script, "w", encoding="utf-8").write(text)

        env = dict(os.environ)
        env["BAA_SPIKE_SCRIPT"] = script
        env["BAA_SPIKE_URL"] = "file://" + os.path.join(HERE, "spike-test.html")
        # A test's own env wins; the ones it does not name are off, so runs do not leak into each other.
        for k in [k for k in env if k.startswith("BAA_HARNESS_")]:
            del env[k]
        env.update(spec.env)
        if spec.ext or spec.profile:
            env["BAA_SPIKE_PROFILE"] = spec.profile or "Default"
        # The mock keychain: no "baa Storage Key" prompt on a runner or after a rebuild.
        args = [exe, f"--user-data-dir={udd}", "--no-first-run", "--enable-logging=stderr", "--use-mock-keychain"]
        if spec.ext:
            args += ["--disable-features=DisableLoadExtensionCommandLineSwitch", f"--load-extension={ext_path}"]
        if spec.cdp_after is not None:
            port = free_port()
            args.append(f"--remote-debugging-port={port}")
        if spec.clipboard_set is not None:
            subprocess.run(["pbcopy"], input=spec.clipboard_set.encode(), check=True)
        clean_downloads()

        with open(log_path, "w") as log:
            started = time.time()
            proc = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT)
            probes = []
            if spec.cdp_after is not None:
                while time.time() - started < spec.cdp_after and proc.poll() is None:
                    time.sleep(0.5)
                for target, expr, regex in spec.cdp:
                    result = run_cdp(port, target, expr)
                    probes.append((expr, regex, result))
                time.sleep(spec.cdp_settle)
                run_cdp(port, "", "Browser.close")
            deadline = started + (spec.kill_after if spec.exit_any and spec.kill_after else spec.secs)
            while proc.poll() is None and time.time() < deadline:
                time.sleep(0.5)
            killed = False
            if proc.poll() is None:
                killed = True
                proc.kill()
            proc.wait()
            code = proc.returncode

        out = open(log_path, errors="replace").read()
        with open(log_path, "a") as log:
            for expr, _, result in probes:
                log.write(f"cdp: {expr} => {result}\n")
        out = open(log_path, errors="replace").read()

        if killed and not spec.exit_any:
            problems.append(f"did not quit within {spec.secs} s (killed)")
        elif not killed and not spec.exit_any and code != 0:
            problems.append(f"exit code {code}, expected 0")
        for pat in [ALWAYS_NOT] + spec.mustnot:
            m = re.search(pat, out, re.M)
            if m:
                line = out[out.rfind("\n", 0, m.start()) + 1 : out.find("\n", m.end())]
                problems.append(f"forbidden /{pat}/ found: {line.strip()[:200]}")
        for pat in spec.must:
            if not re.search(pat, out, re.M):
                problems.append(f"missing /{pat}/")
        for n, pat in spec.count:
            found = [ln for ln in out.splitlines() if re.search(pat, ln)]
            if len(found) != n:
                problems.append(f"expected {n} lines matching /{pat}/, found {len(found)}" + "".join(f"\n      {ln[:200]}" for ln in found[:5]))
        for expr, regex, result in probes:
            if not re.search(regex, result):
                problems.append(f"cdp {expr!r}: /{regex}/ not in {result[:300]}")
        dumps = glob.glob(os.path.join(udd, "Crashpad", "**", "*.dmp"), recursive=True)
        if dumps:
            problems.append(f"{len(dumps)} crash dump(s) in {udd}/Crashpad")
        for rel in spec.exists:
            if not os.path.exists(os.path.join(udd, rel)):
                problems.append(f"missing {rel} under the user data dir")
        if spec.clipboard is not None:
            now = subprocess.run(["pbpaste"], capture_output=True, text=True).stdout
            if now != spec.clipboard:
                problems.append(f"clipboard {now!r}, expected {spec.clipboard!r}")
        if problems:
            interesting = [ln for ln in out.splitlines() if re.search(r"script:|baa: |FATAL|Check failed", ln) and "tab_state" not in ln]
            problems.append("last harness lines:\n" + "\n".join("      " + ln[:220] for ln in interesting[-15:]))
        return problems, tmp
    finally:
        if proc and proc.poll() is None:
            proc.kill()
        # Chrome's helpers outlive a killed browser process.
        subprocess.run(["pkill", "-f", tmp], capture_output=True)
        if server:
            server.kill()
        clean_downloads()
        if not keep and not (locals().get("problems")):
            shutil.rmtree(tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--app", required=True, help="bundled harness app (ci.sh makes it)")
    ap.add_argument("--only", action="append", default=[], help="run only this test (repeatable)")
    ap.add_argument("--keep", action="store_true", help="keep every run's temp dir")
    opts = ap.parse_args()
    exe = os.path.join(opts.app, "Contents/MacOS/baa")
    if not os.access(exe, os.X_OK):
        sys.exit(f"{exe} is not executable")
    if shutil.which("node") is None:
        sys.exit("node is needed for the CDP probes")
    specs = [Spec(p) for p in sorted(glob.glob(os.path.join(TESTS, "*.expect")))]
    if opts.only:
        specs = [s for s in specs if s.name in opts.only]
        if not specs:
            sys.exit(f"no test named {opts.only}")
    failed = []
    for spec in specs:
        t0 = time.time()
        problems, tmp = run_test(spec, exe, opts.keep)
        took = time.time() - t0
        if problems:
            failed.append(spec.name)
            print(f"FAIL {spec.name} ({took:.0f} s)  log: {tmp}/log")
            for p in problems:
                print("  - " + p)
        else:
            print(f"ok   {spec.name} ({took:.0f} s)")
        sys.stdout.flush()
    print(f"\n{len(specs) - len(failed)}/{len(specs)} passed" + (f"; failed: {', '.join(failed)}" if failed else ""))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
