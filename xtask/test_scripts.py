"""python3 -m unittest discover -s xtask -p 'test_*.py'"""
import unittest

import cef_bump
import cef_crate_for
import cef_current
import cef_latest
import privacy_test


def entry(version, channel="stable"):
    return {"cef_version": version, "channel": channel}


class CefLatest(unittest.TestCase):
    def test_newest_stable_present_on_every_platform(self):
        index = {
            "macosarm64": {"versions": [entry("154.0.34+gabc+chromium-1"), entry("153.0.9+gabc+chromium-1"),
                                        entry("155.0.1+gabc+chromium-1", "beta")]},
            "windows64": {"versions": [entry("153.0.9+gabc+chromium-1"), entry("153.0.10+gabc+chromium-1"),
                                       entry("3.3626.1895.g7001d56")]},
        }
        self.assertEqual(cef_latest.latest_common_stable(index, ["macosarm64", "windows64"]), "153.0.9")
        self.assertEqual(cef_latest.latest_common_stable(index, ["windows64"]), "153.0.10")


class CefCurrent(unittest.TestCase):
    def test_reads_cef_dll_sys_build_metadata(self):
        lock = {"package": [{"name": "cef", "version": "154.5.0+154.0.34"},
                            {"name": "cef-dll-sys", "version": "154.5.0+154.0.34"}]}
        self.assertEqual(cef_current.locked_cef(lock), "154.0.34")


class CefCrateFor(unittest.TestCase):
    def test_skips_yanked_and_other_builds(self):
        versions = [{"num": "154.6.0+154.0.35", "yanked": True},
                    {"num": "154.5.0+154.0.34", "yanked": False},
                    {"num": "154.4.0+154.0.33", "yanked": False}]
        self.assertEqual(cef_crate_for.crate_for("154.0.34", versions), "154.5.0+154.0.34")
        self.assertIsNone(cef_crate_for.crate_for("154.0.35", versions))


class CefBump(unittest.TestCase):
    def test_rewrites_only_the_cef_line(self):
        toml = 'cef = "154"\ngpui-cef = { path = "x" }\n'
        self.assertEqual(cef_bump.rewrite_pin(toml, "155.0.0+155.0.3"), 'cef = "155"\ngpui-cef = { path = "x" }\n')
        with self.assertRaises(SystemExit):
            cef_bump.rewrite_pin("serde = \"1\"\n", "155.0.0+155.0.3")


class PrivacyTest(unittest.TestCase):
    def test_only_exact_allowlisted_hosts_pass(self):
        allow = "localhost\neasylist.to\r\nublockorigin.github.io\n"
        hosts = {"easylist.to", "EasyList.to.", "cdn.easylist.to", "localhost", "update.googleapis.com"}
        self.assertEqual(privacy_test.disallowed(hosts, allow), ["cdn.easylist.to", "update.googleapis.com"])

    def test_resolves_the_executable_inside_an_app_bundle(self):
        self.assertEqual(privacy_test.executable("target/bundle/baa.app").parts[-4:],
                         ("baa.app", "Contents", "MacOS", "baa"))
        self.assertEqual(privacy_test.executable("baa.exe").name, "baa.exe")


if __name__ == "__main__":
    unittest.main()
