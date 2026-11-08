# Generated from build_props_config.py.in with the values GN gives a baa build (see ../README.md).
# Copyright 2019 The Chromium Authors
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

from .config import CodeSignConfig


class BuildPropsCodeSignConfig(CodeSignConfig):
    """The BuildPropsCodeSignConfig is processed by GN to inject build-time
    properties from the branding and version data.
    """

    @staticmethod
    def is_chrome_branded():
        return False

    @property
    def enable_updater(self):
        return False

    @property
    def use_static_angle(self):
        return True

    @property
    def app_product(self):
        return 'baa'

    @property
    def product(self):
        return 'baa'

    @property
    def version(self):
        return '156.0.8078.79'

    @property
    def base_bundle_id(self):
        return 'me.baa.browser'

