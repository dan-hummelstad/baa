# Privacy

baa has no server of its own. It sends no telemetry and no usage statistics. This page lists
everything baa contacts without you opening a page, when it does so, and how to turn it off.

## What baa contacts by itself

**Updates (GitHub).** Once a day, baa downloads the update feed
`https://github.com/dan-hummelstad/baa/releases/latest/download/appcast.xml` from GitHub. When a
new version exists, it downloads the DMG from GitHub too. Turn it off: Settings › System ›
Automatic updates. "Check for Updates" on the same page still works by hand.

**Search suggestions (your search engine).** While you type in the command bar (⌘T or ⌘L), baa
sends what you type to your default search engine to get suggestions. The default search engine
is Google, and suggestions are on by default. Turn it off, or pick another engine:
Settings › Search › Suggestions while typing. With suggestions off, typing contacts nobody.

**Extension updates (Google).** In builds that have the setting, baa checks Google's Chrome Web
Store update server about every five hours for new versions of your extensions. The check sends
the ids and versions of the extensions you installed. Turn it off: Settings › Privacy › Update extensions automatically. Builds
without the setting never update extensions.

**Certificate revocation list (Google).** In builds that have the setting, baa downloads
Chromium's list of revoked certificates (CRLSet) from Google's update server. Turn it off:
Settings › Privacy › Update certificate revocation list (applies after a restart).

**DRM video (Google).** Only when "Play DRM-protected content" is on (it is off by default), baa
downloads the Widevine module from Google's update server. Turn it off: Settings › Privacy ›
Play DRM-protected content (applies after a restart).

**Live folders (the sites you chose).** A live folder fetches the feed or page you added every
ten minutes, and when you choose Refresh. A GitHub live folder calls GitHub's API with your own
GitHub token (`GH_TOKEN`, `GITHUB_TOKEN`, or the GitHub CLI's login, `gh auth token`). The token
goes only to GitHub. Turn it off: delete the folder.

**uBlock Origin's filter lists.** The default lists are bundled with baa and update with baa. They
are not downloaded. A list that you add in uBlock Origin's settings is downloaded from that list's
own site. Turn it off: remove the list.

**Claude (Anthropic).** Off by default and experimental. When you turn it on in Settings › Claude,
baa runs your own `claude` command-line tool, signed in with your own account. What you ask
Claude, and the pages you let it read, go to Anthropic through that tool. baa adds no account or
server of its own. Turn it off: Settings › Claude › Let Claude use baa.

**Mail (your mail provider).** Off by default and experimental. When you turn it on in
Settings › Mail and add an account, baa connects to that account's mail servers, and to no other
host: it keeps one connection open to hear of new mail, checks the other folders every ten
minutes, and connects when you read, move or send a message. Your password, or the sign-in token
of a Google account, goes only to that provider, and is kept in the macOS Keychain. Mail you have
read is kept on your Mac in `~/Library/Application Support/baa/mail/`, not encrypted by baa.
Images that a message loads from the web are not fetched until you ask for them. A verification
code is read from your mail on your Mac and goes into a page only when you click it. baa has no
mail server of its own and sends your mail nowhere else. Turn it off: remove the account, or
Settings › Mail › Mail.

Extensions you install can contact any site they have permission for. That is up to each extension.

## What baa does not do

- No baa server and no baa account.
- No telemetry, analytics or usage statistics.
- No crash upload. Crash dumps stay on your Mac in
  `~/Library/Application Support/baa/chromium/Crashpad/`.
- No Safe Browsing. baa does not check sites against Google's lists, so it does not warn about
  phishing or malware sites. Downloads are judged by file type only.
- No sync. Your history, tabs and settings stay on your Mac. Mail accounts are not carried to
  another device either.

## Checked by CI

Before each release is published, CI starts the signed app behind a proxy and fails the release if
baa contacts a host it does not expect (`crates/baa-privacy`). The proxy sees what the browser
engine contacts. It does not see the connections baa makes itself for live folders and mail;
those are held to the servers you chose by the code and its tests.
