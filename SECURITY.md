# Security policy

## Reporting a vulnerability

Report it privately through GitHub:
<https://github.com/dan-hummelstad/baa/security/advisories/new>.
Do not open a public issue for a security problem.

Include what you found, the baa version (the release you installed), and the
steps to reproduce it.

## Scope

In scope is baa's own code:

- the shell (the Rust app in `crates/`),
- `//baa`, the layer between the shell and Chromium (`chromium/baa`),
- baa's patches to Chromium (`chromium/patches`),
- the build and release pipeline (`.github/`, `xtask/`).

Not in scope here:

- A bug in Chromium itself: report it to Chromium at <https://issues.chromium.org>.
- A bug in a Helium patch (`chromium/helium*`): report it to Helium at
  <https://github.com/imputnet/helium>.

If you are not sure where a bug belongs, report it here; it will be forwarded.

## Supported versions

Only the latest release gets fixes. Update before you report.

## What to expect

- An acknowledgement of your report.
- A fix in a release, and an advisory when that release is out.
- Credit in the advisory if you want it; say how you want to be named.
