"""python3 -m unittest discover -s xtask -p 'test_*.py'"""
import unittest

import cef_current
import privacy_test


class CefCurrent(unittest.TestCase):
    def test_reads_cef_dll_sys_build_metadata(self):
        lock = {"package": [{"name": "cef", "version": "154.5.0+154.0.34"},
                            {"name": "cef-dll-sys", "version": "154.5.0+154.0.34"}]}
        self.assertEqual(cef_current.locked_cef(lock), "154.0.34")


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
