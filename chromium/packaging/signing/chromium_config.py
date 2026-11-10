# Copyright 2019 The Chromium Authors
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import os

from signing.build_props_config import BuildPropsCodeSignConfig


class ChromiumCodeSignConfig(BuildPropsCodeSignConfig):
    """A CodeSignConfig used for signing non-official Chromium builds.

    This is primarily used for testing, so it does not include certain
    signing elements like provisioning profiles.
    """

    @property
    def provisioning_profile_basename(self):
        if os.environ.get('PROD_MACOS_SPECIAL_ENTITLEMENTS_PROFILE_PATH'):
            return 'Helium'
        return None

    @property
    def run_spctl_assess(self):
        # security: Gatekeeper's assessment moves, it is not dropped. Here it runs
        # before notarization and rejects every Developer ID app ("Unnotarized
        # Developer ID"); release.yml runs spctl --assess on the stapled app instead.
        return False
