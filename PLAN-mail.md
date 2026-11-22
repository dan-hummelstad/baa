# baa Mail — plan

Status: **approved** (user, 2026-11-22); stage 0 is under way. §14 records the answers and the one
question that is still open.

Basis: `main` at `57ff25b7` (baa 0.1.9, ABI 16, Chromium 158). `file:line` references were read at
that commit and drift; search for the name if a line does not match.

Design source: the pen.dev file `baa.pen`, frame `R2fgxx` "3. baa Mail — Take 3". Six screens, the
same three in each skin: A1/B1 the Archive with its Mailbox door, A2/B2 the Mail section (list and
reading pane), A3/B3 compose in place with draft tabs. Agents read them with the pencil MCP tools
(`Get("R2fgxx", …)`, `Export`), never with Read or Grep.

User decisions so far (they win over anything below):
- 2026-11-22: baa registers its own OAuth clients with Google and Microsoft.
- 2026-11-22: verification-code autofill from mail (as Safari does with Apple Mail) is part of the
  first spike (§7).
- 2026-11-22: the plan's recommendations stand (§14). Mail protocols outside Chromium are allowed
  on their standard ports only (§9). The engine build for codes is part of the first spike. The
  OAuth code is built as if the client ids exist; the user registers the clients outside this work.
- 2026-11-22: who does the work (§12): the orchestrator, up to five Opus 5.5 leads, and up to three
  Sonnet 5.5 workers under each lead.

---

## 1. What is built

A mail client inside baa. It is a client only: baa talks to the user's mail servers and to nobody
else, and there is no baa service. Mail accounts belong to a Space (or to every Space). The Archive
has a Mailbox door; opening it swaps the sidebar for mail navigation and the page area for the mail
view. Message bodies and the compose editor are Chromium pages; everything around them is GPUI, in
both skins. Mail can be one pane of a split beside browser tabs. Each Space remembers whether it
showed the browser or mail, and what mail showed.

**The first spike (stages 0 and 1) is done when**, with Settings › Mail turned on:
- An account is added with an app password (iCloud, Fastmail, any IMAP server) or with Google
  sign-in (Gmail), to the current Space or to every Space.
- The inbox syncs and stays current (IMAP IDLE), several accounts of a Space show as one "All
  inboxes" list, and the scope chips filter to one account.
- An HTML message renders in the reading pane with its scripts dead and its remote images not
  loaded until the user allows them; a link opens as a tab of the Space.
- New message, reply, reply all and forward compose in place as draft tabs, with bold, italic,
  underline, lists and links, attachments, autosave to Drafts, and send.
- Delete, archive, move, flag, mark read or unread, and search work, and reach the server.
- ⌘⇧M goes between browser and mail and each Space comes back to what it showed; a Space switch
  inside mail shows that Space's accounts; mail sits in a split beside a tab; a restart restores all
  of it.
- A page's one-time-code field offers the code from a just-arrived message of the Space's accounts,
  and fills it on one click (§7).
- An incognito Space or window has no mail, no code offers, and writes nothing.

---

## 2. What the mocks show, and how each part is built

| Mock | Part | Built as | Stage |
|---|---|---|---|
| A1/B1 | Archive rail: Mailbox first, then Downloads, Archived Tabs, Chat | `library::Section::Mailbox`, first `nav_item` (`library.rs:297`) | 1 |
| A1/B1 | Mailbox door: "All inboxes" card with the Space's accounts and counts, Open Mailbox | the section's body, as Chat has its own (`library.rs:317`) | 1 |
| A1/B1 | "Waiting for you" (unread, flagged, snoozed, scheduled, mentions), "Recent files" | rows under the door; each opens mail on that view | 1 unread/flagged, 3 rest |
| A2/B2 | Sidebar becomes mail nav: account header, New Message, Folders, Search folders, Categories, Other Spaces | `render_sidebar` early return, as the Library has (`sidebar.rs:65`) | 1 (categories 3) |
| A2/B2 | Section switcher Browser · Archive · Mail above the Space strip | a row in `render_bottom_bar` (`sidebar.rs:960`) | 1 |
| A2/B2 | Tab strip: Inbox plus one tab a draft | mail view, GPUI | 1 |
| A2/B2 | Command row (ASCII: ribbon tabs Home/View/Folder/Options; Arc: pills) | one command table, drawn per skin | 1 the Home set, 3 rest |
| A2/B2 | List: Focused/Other, scope chips, Pinned/Today/Yesterday groups, account dot a row | `gpui::uniform_list` (precedent `space_picker.rs:750`) | 1 (Focused/Other, pinned 3) |
| A2/B2 | Reading pane: subject, chips, sender, body, attachments, earlier messages, quick reply | GPUI around a message page (§6) | 1 (chips 3) |
| A3/B3 | Composer in the reading pane: From, To, Cc, Bcc, Subject, options row, attachments, body, footer | GPUI fields (`TextField`) around an editor page (§6) | 1 (options row 3) |
| A3/B3 | Editor ribbon: Message, Insert, Format text, Options, Review | commands to the editor page | 1 basic set, 3 rest |
| A2 | Status line: "imap ✓ synced 40s ago · 12 unread" | mail view footer | 1 |

In the mocks and **not planned**: the Draw tab, Loop cloud attachments, Translate, Read aloud, the
calendar and bell buttons, the Clipboard and History rail items, "Encrypt" (S/MIME), Microsoft 365
Groups. They need a service, a licence or a product of their own. §10 (stage 4) and §15 say which could
come later.

The mock's captions say the whole mail surface is one Chromium page (`baa://mail`). The brief says
baa's own UI elements with Chromium rendering the messages; the plan follows the brief (decision 1).

---

## 3. Decisions

| # | Question | Decision |
|---|---|---|
| 1 | What draws the mail surface? | GPUI draws all of it; Chromium draws a message body and the compose body only. The brief says so, the sidebar is GPUI anyway, and one more WebUI would need an engine build for every change. |
| 2 | Where does the code live? | `crates/baa-mail` holds everything that is not a view: accounts, sync, store, MIME, send, rules, code extraction. No gpui, as `baa-model` has none. The views live in `crates/baa-ui/src/mail/`. A `baa-mail` that held views would need `baa-ui`'s widgets (all private today, `baa-ui/src/lib.rs:3-24`) while `baa-ui`'s `Browser` needs the views: a dependency cycle. It also keeps most of the work in a crate that builds in seconds. The other way (views in `baa-mail`, `baa-ui` exporting its widgets plus a slot for foreign views) costs a slot contract and theme plumbing and gains nothing the user sees. |
| 3 | Is Mail a tab? | No. A Space shows the browser or mail (`Space.showing`), and a split pane is a tab or mail (`Pane`). A hidden pseudo-tab would need a guard in every piece of tab code (archive, ⌘W, ⌘1–8, the command bar, the agent's tools), none of which the compiler would find. |
| 4 | Where does mail data live? | `<data dir>/mail/mail.db`, opened by `baa-mail`. Not `baa.db`: mail is large, is written from sync threads (`Store` is one connection on the UI thread, `store.rs:36`), and is a cache that can be dropped. |
| 5 | Where do secrets live? | The macOS Keychain, one generic-password item an account and kind, service `me.baa.mail`. Never in `mail.db`, `baa.db`, a log or argv. baa has no Keychain code today; this is the first. |
| 6 | Which network stack? | IMAP: a Rust library over `rustls`, on one thread an account (IDLE holds a connection open; curl cannot). SMTP, POP3 and every HTTPS call (OAuth, Graph, autoconfig): the system `curl`, as live folders use it (`baa-model/src/live.rs:290`), secrets on stdin. No async runtime, no HTTP client crate. |
| 7 | How is untrusted HTML shown? | In a sandboxed frame under a content security policy, inside a page that has no URL (§6). The browser's own isolation is the control; baa does not write an HTML sanitizer. |
| 8 | Which profile holds a message page? | The Space's. A link then opens through the engine's own new-tab path (adopted, never re-issued) in the storage the user is signed in to. Remote images are off by default, so the Space's cookies go nowhere unless the user loads them. |
| 9 | Mail in incognito? | None. No door, no accounts, no code offers, no writes. |
| 10 | When does baa sync? | Only while baa runs, and only accounts the user added. INBOX of every account is held with IDLE; other folders are polled every 10 minutes and when opened. |
| 11 | Microsoft accounts | Microsoft Graph for Outlook.com and Microsoft 365, never IMAP: Graph has shared mailboxes, rules, categories and Focused, and is https. Stage 2. |
| 12 | Gmail | IMAP and SMTP with XOAUTH2 (scope `https://mail.google.com/`), on the IMAP backend. Stage 1, since codes matter most there. |
| 13 | Exchange on-premises | EWS over curl (NTLM or Kerberos), stage 4, optional. Until then such a server works through IMAP where its admin allows it. |
| 14 | Proton, iCloud | Proton: only through Proton Mail Bridge on 127.0.0.1 (a preset; its ports are not the standard ones, so it waits for §14's open question). iCloud: IMAP with an app-specific password (a preset). Neither offers third parties anything else. |
| 15 | Trunk and releases | All of it lands on `main` behind `Settings.mail.enabled`, off by default, so releases go on. The default changes when the user says the spike is good. |
| 16 | Keys | ⌘⇧M browser ⇄ mail (free today). Inside mail: ⌘N new message, ⌘R reply, ⌘⇧R reply all, ⌘⇧F forward, ⌘↩ send, ⌫ delete, ⌘F search mail. Mail has its own key context, so ⌘N and ⌘R mean New Window and Reload only outside it. |
| 17 | Skins | No mail theme. The views take `&Theme` and branch on `theme::ascii()` as the rest of `baa-ui` does (`sidebar.rs:24`). ASCII draws the ribbon, Arc draws pills, from one command table. |

---

## 4. Architecture

```
crates/baa-mail  (no gpui)                      crates/baa-ui/src/mail/  (views)
  account  provider  secrets                      nav · list · reader · compose · tabs
  backend: imap | graph | pop3 | fake             commands · setup · code_offer
  sync (one thread an account)   ── Event ──►   Browser<H>: showing, panes, sidebar swap
  store (mail.db, FTS5)          ◄── reads ──   (UI thread reads mail.db directly)
  mime  send  rules  codes       ◄── Action ─
                                                crates/baa-web: create_document, code field (ABI 17)
```

**Threads.** `Mail::open` starts nothing. Each account gets one thread when it is first needed; the
thread owns the backend and a write connection. The UI thread has a read connection (WAL) and reads
synchronously, as it reads `Store`. Actions go to the account's thread over `std::sync::mpsc`; the
thread applies them to `mail.db` first, so the list changes at once, then to the server, and queues
them when offline. Events come back through the `notify` closure given to `Mail::open`; `baa-ui`
forwards them over `futures::channel::mpsc` into a `cx.spawn` loop (precedent `agent_panel.rs:486`).

**Store.** Tables: `account`, `folder`, `message` (envelope, flags, thread, snippet, auth result),
`body` (the raw RFC 822 bytes, fetched on first open), `attachment` (index into the body),
`message_fts` (FTS5: subject, people, text), `contact`, `draft`, `outbox`, `op` (unsent actions),
`local` (what no server holds: pin, snooze, category colours, rules, signatures), `view` (one JSON
row a Space). `PRAGMA user_version` as `Store` does (`store.rs:40`): a newer file is refused; an
older one drops and refills the cache tables and migrates `draft`, `outbox`, `op`, `local`, `view`.

**Backends.** `Backend` is a trait from the start because it has two implementations on day one
(`Imap` and the in-memory `Fake` every UI test runs on) and four by stage 2.

| Provider | Backend | Sign-in | Stage |
|---|---|---|---|
| Any IMAP server, Fastmail, Yahoo | imap + smtp | password or app password | 1 |
| iCloud | imap + smtp (preset) | app-specific password | 1 |
| Gmail, Google Workspace | imap + smtp (preset: labels, All Mail, `X-GM-THRID`) | Google OAuth, XOAUTH2 | 1 |
| Proton | imap + smtp to Proton Mail Bridge on 127.0.0.1 (preset) | Bridge password | 2 |
| Outlook.com, Microsoft 365 (Exchange Online) | graph | Microsoft OAuth | 2 |
| Shared mailbox (Microsoft 365) | graph, added by address | the user's own sign-in | 2 |
| POP3 | pop3 (curl) + smtp | password | 2 |
| Exchange Server on-premises | ews (curl) | NTLM, Kerberos | 4 |
| JMAP, ManageSieve | — | — | 4, if asked for |

**OAuth.** baa is the browser, so the consent page opens as a tab of the Space, where the user is
probably signed in already. Authorization code with PKCE (S256) and `state`; the redirect is
`http://127.0.0.1:<port>/` on a `std::net::TcpListener` that binds loopback only, takes one request,
checks `state`, and closes after 5 minutes. The token exchange and refresh are curl to https. The
client ids are constants in `baa-mail/src/oauth.rs`, empty until the user has registered the
clients (`BAA_GOOGLE_CLIENT_ID` and `BAA_MICROSOFT_CLIENT_ID` in the environment override them). With
no id the provider's sign-in button says so and does nothing; no test needs a real client.

**Dependencies to add** (all in `crates/baa-mail`; each is confirmed and its licence checked against
`about.toml` in WP-P2, since `cargo about` runs only in `release.yml`):

| Crate | For | Instead of |
|---|---|---|
| `rustls`, `rustls-platform-verifier` | TLS 1.3 for IMAP, trust from the system store | `native-tls` (Secure Transport: deprecated, TLS 1.2 at most) |
| `imap-next` on `imap-codec` (fallback: `imap` 2.4) | IMAP without an async runtime | curl (no IDLE, one login a command) |
| `mail-parser` | MIME parsing of untrusted mail, HTML to text | writing one |
| `mail-builder` | building outgoing MIME | `lettre` (brings an SMTP client we do not use) |
| `security-framework` | Keychain items (already in the graph through the verifier) | the `security` CLI (the secret would be on argv) |

---

## 5. The shell: sections, sidebar, split, memory, gestures

Owner: `baa-ui` (the files named here), with the model change in `baa-model`.

- **Showing.** `Space.showing` is `Browser` or `Mail`. `render_sidebar` returns the mail nav when the
  focused pane is mail (the Library's early return still comes first). `render_page`
  (`page_area.rs:51`) draws the mail view in place of the page; `show_pages` (`page_area.rs:401`)
  hides the Space's native pages while it does.
- **Door.** `Section::Mailbox` heads the Library rail. Its body is the door of mock A1/B1. Open
  Mailbox closes the Library, restores the 228 px sidebar and sets `showing = Mail`. In an ephemeral
  Space the rail item is not drawn. With no account the door is the "Add an account" card.
- **Switcher.** Browser · Archive · Mail, one row above the Space strip, drawn only when
  `settings.mail.enabled`. ⌘⇧M goes between Browser and Mail; ⌘⇧A stays the Archive.
- **Gesture.** Today the Archive opens by a swipe right past the first Space (`nav.rs:99`). New: a
  two-finger swipe up over the bottom bar opens it in any Space, and a swipe down closes it. The bar
  does not scroll, so the vertical axis is free there. Horizontal swipes keep switching Spaces, in
  mail too: the nav slides as the tab list does and the mail view follows the Space.
- **Other Spaces.** The mail nav lists the other Spaces that have accounts, with unread counts; a
  click goes to that Space with mail showing. The door lists them too.
- **Split.** `Split.panes: Vec<Pane>` replaces `tabs: Vec<TabId>`. ⌃⇧= in mail splits mail with the
  most recent tab; dragging a tab onto the mail view adds it; the pane strip's × takes mail out. The
  sidebar follows the focused pane (`PageEvent::Focused` for a tab, a click for mail). At most one
  mail pane a Space. A split holding a mail pane shows when the Space shows mail or its active tab is
  one of the panes.
- **Memory.** `Space.showing` and `Space.active_tab` are in the session already or by this change.
  What mail showed (folder, scope, sort, selected conversation, open draft tabs and which is in
  front, list scroll) is the `view` row of `mail.db`, written on change and read on Space switch and
  at launch.
- **Space delete.** The confirmation names the mail accounts that go with the Space, as it names
  extensions (`settings/tabs.rs:612`); then `mail.remove_accounts_of(space)` drops their rows, cache
  and Keychain items. Accounts of every Space stay.
- **Native views.** A message or editor page is a native view under gpui's layer: anything drawn over
  it must `.occlude()` (`page_area.rs:171`), and menus are native (`host.show_menu`).

---

## 6. Message and editor pages

Owner: `baa-web` (Rust only; no engine change). The whole policy is one module,
`core_host/document.rs`, so it is reviewed and tested in one place.

A **document** is a page baa writes itself. It starts as `about:blank` (opaque origin, not recorded
in history) and never navigates. `set_document` writes it by script from the browser side.

**Message** (`DocumentKind::Message { remote_images }`):
- The page holds a policy and one frame: `default-src 'none'; img-src data:; style-src
  'unsafe-inline'; frame-src 'none'; form-action 'none'` (`img-src data: https:` once the user allows
  remote images for the message or its sender).
- The message's HTML is the frame's `srcdoc`. The frame is `sandbox="allow-popups
  allow-popups-to-escape-sandbox"`: no scripts, no forms, no same origin, no navigating the page.
- `<base target="_blank">` goes first, so a link opens through the engine's own new-tab path and
  arrives as `PageEvent::OpenRequested`, which the shell adopts. baa never loads a URL taken from a
  message.
- `cid:` images become `data:` URLs before the HTML is handed over (`baa-mail`, from the parsed
  parts). A plain-text message is escaped into a `<pre>` by `baa-mail`.
- `PageEvent::TargetUrl` (exists) drives the "where this link goes" line under the reading pane.

**Editor** (`DocumentKind::Editor`): the same policy plus `img-src data: blob:`, and a
`contenteditable` body in the page itself. Page scripts and inline handlers stay dead under the
policy, so quoted mail is inert in it; the shell's own `eval` is not subject to it. Formatting is
`document.execCommand` through `PageHandle::eval`; the content is read with `eval_value`
(`innerHTML` for the HTML part, `innerText` for the plain part). Chromium gives spelling, IME, undo
and paste for nothing. `ponytail:` the toolbar does not show the selection's state in stage 1; the
ceiling is a page-to-host channel, which ABI 17 could carry.

**Attachments.** Saving writes to the download folder with the file name reduced to its last
component, sets the quarantine attribute as a browser download has, and records a
`DownloadRecord` so the file shows in the Library. Nothing opens or runs an attachment from mail;
executable types get the same warning the download tray gives a dangerous download.

**Proof (WP-P1).** A harness test on the release engine (CI runs it): a document whose message has a
`<script>`, an `onerror`, a remote `<img>`, a `<form>`, a `<meta refresh>` and a link. Expect no
script effect, no request at the test server, no navigation, and one open request for the link.
Fallback if a document cannot be written this way: a `baa://mail` data source in `//baa` with the
same policy as a response header, in the ABI 17 batch.

---

## 7. Verification codes from mail

The pasted brief was reviewed against baa. What it asks is kept; how it gets there changes.

| The brief | In baa |
|---|---|
| Step 0: choose Chromium, Gecko or a shell; write `docs/DECISION.md` | Settled: baa is its own Chromium with `//baa`. No decision file. |
| Link mailboxes in settings, read-only scopes (`gmail.readonly`, `Mail.Read`) | A mail account of the Space is the link; it has full mail access already. Each account has a "Offer verification codes" switch. No second, read-only way to link. |
| The suggestion appears in the standard Autofill dropdown | Helium removes Chrome's autofill UI (`PLAN-settings.md` row 11), and baa draws Chrome's prompts itself. baa draws the offer, anchored under the field, and it follows the security-prompt rules. |
| Poll only while a code field is focused; never sweep in the background | Mail syncs anyway. A code field asks the Space's accounts for one immediate INBOX check; IDLE brings the rest. No extra polling exists to limit, and a page can cause at most one check every 10 seconds an account. |
| Gmail and Microsoft first, IMAP second | The extractor reads parsed messages, so it serves every backend. Stage 1 has IMAP and Gmail; Microsoft arrives with Graph in stage 2. |
| Do not store codes | The message is in `mail.db` like any mail. The code path stores nothing of its own: no table, no log line with a code, a sender or an address. |
| Keychain on three systems | macOS only, as baa is. |
| `docs/THREATS.md`, fuzz tests | The threat table below; a mutation test over a corpus in `baa-mail` (`ponytail:` no fuzzing toolchain; add `cargo fuzz` if the extractor grows). |
| Phases 1–4, stop after each | Folded into stage 1 (IMAP, Gmail) and stage 2 (Graph, alias table, clean-up setting). |

**Flow.**
1. The engine reports that a field for a one-time code has focus in a tab: `autocomplete=
   "one-time-code"`, or Chromium's own classification of the field. It gives an id, the frame's
   origin, the top-level origin and the field's rectangle (`PageEvent::CodeField`, ABI 17).
2. The shell drops the report unless: the Space is not ephemeral, mail and codes are on, the Space
   has an account with codes on, and the frame is the main frame or same-site with it.
3. `mail.check_now(space)`, then for 2 minutes each `Event::Changed` re-runs
   `mail.codes(space, since = field time − 60 s)`. Only messages that arrived in the last 10 minutes
   count.
4. Extraction, in this order: the origin-bound last line (`@host #code`, WICG), the `OTP-Token`
   header, then the patterns in `baa-mail/src/codes.toml` (4–8 digits or a short alphanumeric token
   beside a phrase such as "verification code"). The input is the text part, or the HTML reduced to
   text by the parser; nothing is rendered.
5. A code is offered only if the sender matches the page. `@host` must be the page's site. Otherwise
   the sender's registrable domain must be the page's, or an alias in `codes.toml`
   (`accounts.google.com` ← `google.com`, and so on). The engine gives the registrable domain
   (`WebHost::site_of`), so it is Chrome's notion, not a second list. A message whose
   `Authentication-Results` from the user's own provider says DMARC failed is never offered. No
   match: nothing is shown.
6. The offer reads "Fill code 123456 · from no-reply@site.com · you@example.com". A click fills it
   through the engine (`PageHandle::fill_code`), which checks the field and its origin again. Nothing
   submits the form. The offer goes when the field loses focus, the tab navigates, or 2 minutes pass.
7. Off by default, per account: after a code is used, mark its message read or move it to Trash.

**Threats.**

| Threat | Control |
|---|---|
| A page probes for codes | The page learns nothing: detection and the code stay in the browser process, and the page sees a code only after the user's click. |
| A page forces mailbox traffic | One check every 10 s an account, a 2-minute window a field, and nothing in a background tab. |
| A look-alike domain | Site match on the registrable domain from the engine; the offer shows the engine's origin (`host_str`, cut on the left) and the sender's real address. |
| A field in a cross-site frame | Not offered. |
| A click meant for the page lands on the offer | Input is ignored for 500 ms after the offer appears or changes; Enter and key repeat never accept; element ids come from the field id. |
| A forged mail carrying a code | DMARC failure hides it; a wrong code is the worst outcome, since the page already asked for one. |
| Hostile mail content | A hardened parser, text only, mutation-tested; never rendered on this path. |
| Tokens on disk | Keychain only. |
| A compromised renderer | It can claim a code field exists, which a page can do anyway. It cannot read mail state, and the fill goes only to the frame the browser process chose. |

**Engine (ABI 17, one batch, built in CI).** `code_field` callback and `baa_tab_fill_code` in a new
`chromium/baa/codes.mm`, and `baa_site_of`. The engine WP first reads Chromium 158's autofill code
to choose the hook: its one-time-code field type if forms are still classified with address and card
autofill off, else a focused-element check in the browser process. This is the one engine build of
the spike; everything else codes against `MockHost` until it lands.

---

## 8. Contracts

WP-0 lands these as stubs with defaults; after that only the orchestrator changes them.

**baa-model** (`lib.rs`, `settings.rs`)
```rust
/// What one pane of a split holds. A bare number is a tab, so sessions saved before mail load.
#[serde(untagged)]
pub enum Pane { Tab(TabId), Mail(MailPane) }          // MailPane: a unit marker that serializes as "mail"
// The JSON key stays `tabs`: a session with no mail pane is what 0.1.9 wrote, and 0.1.9 can read it.
pub struct Split { #[serde(rename = "tabs")] pub panes: Vec<Pane>, pub widths: Vec<f32> }

pub enum Showing { Browser, Mail }                    // tolerant: unknown → Browser
pub struct Space { /* … */ #[serde(default)] pub showing: Showing }

pub struct MailSettings {                             // Settings.mail, #[serde(default)]
    pub enabled: bool,                                // false
    pub remote_images: RemoteImages,                  // Ask (default) | Never; tolerant → Never
    pub codes: bool,                                  // true; nothing happens without an account
}
```
Private mode: an ephemeral Space is never saved, so `showing` and `Pane::Mail` are not; test both.

**baa-web** (`lib.rs`; `MockHost`/`MockPage` record every call)
```rust
pub enum DocumentKind { Message { remote_images: bool }, Editor }

// WebHost
/// A page baa writes itself (§6 of PLAN-mail.md): it has no URL and never navigates.
fn create_document(&self, profile: ProfileId, kind: DocumentKind, window: &mut Window, cx: &mut App) -> Entity<Self::Page>;
/// The registrable domain of `host` ("mail.google.com" → "google.com"), as Chrome sees it.
fn site_of(&self, host: &str) -> String;                                   // ABI 17

// PageHandle
/// Replaces a document's content. Does nothing on a page that has a URL.
fn set_document(&self, html: &str);
/// Fills the code field `id` if it still has focus and its origin is unchanged.
fn fill_code(&self, id: u64, code: &str);                                  // ABI 17

// PageEvent
/// A one-time-code field has focus. `origin` is its frame's, `top` the page's, `at` its
/// rectangle in the page. `None` in `at`: field `id` lost focus or is gone.
CodeField { id: u64, origin: String, top: String, at: Option<gpui::Bounds<gpui::Pixels>> },   // ABI 17
```

**baa-mail** (`lib.rs`)
```rust
pub struct Mail;                                       // Send + Sync; one a process, shared by windows
impl Mail {
    pub fn open(dir: &Path, secrets: Box<dyn Secrets>, notify: Box<dyn Fn(Event) + Send + Sync>) -> Result<Mail>;
    // Accounts. `space: None` on an account means every Space. No call takes an ephemeral Space.
    pub fn accounts(&self, space: SpaceId) -> Vec<Account>;
    pub fn add_account(&self, new: NewAccount) -> Result<AccountId>;      // tests the sign-in first
    pub fn remove_account(&self, id: AccountId);
    pub fn remove_accounts_of(&self, space: SpaceId) -> Vec<Account>;
    pub fn other_spaces(&self, than: SpaceId) -> Vec<(SpaceId, u32 /* unread */)>;
    // Reading: synchronous, from mail.db, on the caller's thread.
    pub fn folders(&self, space: SpaceId) -> Vec<Folder>;
    pub fn rows(&self, view: &View, range: Range<usize>) -> Vec<Row>;     // one row a conversation
    pub fn count(&self, view: &View) -> usize;
    pub fn conversation(&self, id: ThreadId) -> Vec<Message>;
    pub fn body(&self, id: MessageId) -> Option<Body>;                    // None: asked for, Event follows
    pub fn search(&self, space: SpaceId, text: &str) -> View;
    // Changing: applied to mail.db at once, then to the server.
    pub fn act(&self, on: &[MessageId], action: Action);                  // Read(bool) Flag(bool) Move(FolderId) Archive Delete
    pub fn save_draft(&self, draft: &Draft) -> DraftId;
    pub fn send(&self, draft: DraftId);
    pub fn save_attachment(&self, of: MessageId, part: usize, dir: &Path) -> Result<PathBuf>;
    // Per-Space view memory.
    pub fn view_state(&self, space: SpaceId) -> ViewState;
    pub fn set_view_state(&self, space: SpaceId, state: &ViewState);
    // Codes (§7).
    pub fn check_now(&self, space: SpaceId);
    pub fn codes(&self, space: SpaceId, since: SystemTime) -> Vec<Code>;  // sender, account, code, bound host, dmarc
}
pub enum Event { Changed { account: AccountId }, Status { account: AccountId, state: SyncState }, Sent(DraftId), Failed { account: AccountId, what: String } }
pub trait Secrets: Send + Sync { fn get(&self, account: AccountId, kind: Secret) -> Option<String>; fn set(&self, account: AccountId, kind: Secret, value: &str); fn delete(&self, account: AccountId); }
pub trait Backend: Send { /* sync, wait (IDLE), apply, fetch_body, send: filled in by WP-0 */ }
```
`Failed.what` never holds a host, an address or a secret above debug level (`browser/live.rs:109`).

**baa-ui** (`mail/mod.rs`): `Browser<H>` gains `mail: Option<MailUi<H>>` (`None` in a private window
or with mail off). `MailUi` holds the `Arc<Mail>`, the Space's view state, the open documents and
the draft tabs. It exposes `render_nav(&self, theme, window, cx)`, `render_view(&self, theme,
window, cx)` and `render_door(...)`, methods on `Browser<H>` as the Library's are. Tests build it on
`Mail::open` with the `Fake` backend, in-memory secrets and `:memory:`.

**//baa** (`baa.h`, ABI 17): `code_field(ctx, tab, id, origin, top, x, y, w, h, shown)` in
`BaaCallbacks`; `baa_tab_fill_code(tab, id, code)`; `baa_site_of(host, out, len)`.

---

## 9. Security and privacy rules for this work

Additions to CLAUDE.md "Security invariants". The first changes an existing invariant; the user
approved it on 2026-11-22, for the standard ports only.

- **Mail connections are the one exception to "http(s) only".** `baa-mail` connects by IMAP, POP3
  and SMTP to the servers of an account the user added, and to no other host, on the standard ports
  only: IMAP 993 or 143, POP3 995 or 110, SMTP 465 or 587. Any other port is refused. Always TLS
  (implicit, or STARTTLS that must succeed); the certificate and host name are checked against the system
  trust store; there is no "continue anyway". curl for SMTP and POP3 passes `--proto` and
  `--proto-redir` naming only those schemes, and `--ssl-reqd`. Every other fetch in `baa-mail` is
  https, checked on every URL.
- **Mail content is untrusted.** HTML from a message reaches Chromium only through
  `create_document`. baa never loads, fetches or opens a URL, file name or header value taken from
  a message; a link opens only by the user's click, through the engine's adopted tab.
- **Mail secrets are in the Keychain only**, and never on argv.
- **No mail in incognito**: no account, no code offer, no write to `mail.db` or the Keychain from an
  ephemeral Space or a private window. Every `Mail` call that takes a Space has the test.
- **A code offer is a security prompt** and follows those rules (origin from the engine, 500 ms,
  no Enter, ids from the request).
- **Attachments are downloads**: quarantined, never opened by baa, dangerous types warned about.

`security-lint.sh` gains: a `Command::new("curl")` in `crates/baa-mail` must sit beside
`--ssl-reqd` or `=https`; no `danger`/`dangerous` rustls configuration anywhere.

PRIVACY.md gains an entry in the form of the live-folders one: baa contacts a mail server only for
an account you added, when (IDLE, every 10 minutes, on your actions), what goes where (your
password or token to your own provider only), and how to stop it (remove the account, or turn Mail
off). It also says the privacy test does not see these connections: it proxies Chromium only. The
"No sync" line gets "mail accounts are not synced between devices either".

---

## 10. Stages

Each ends runnable, with mail still off by default.

| Stage | Name | Holds | Needs from the user |
|---|---|---|---|
| 0 | Contracts and proofs | WP-0, WP-P1, WP-P2 | — |
| 1 | Read, write, manage, codes | WP-A … WP-N (§11). Ends at §1's list. | a Google OAuth client id, to try Gmail for real |
| 2 | More accounts | Microsoft Graph (Outlook.com, Microsoft 365, shared mailboxes, send as), POP3, Proton Bridge preset, autoconfig from the domain's own records and its MX, codes on Graph, the alias table, share an account to every Space from Settings | an Entra app id (and publisher verification before release) |
| 3 | Outlook's management | categories, follow-up flags with dates, pin, snooze, scheduled send and outbox, Focused/Other, search folders, sweep, quick steps, rules (local, then the server's on Graph and Gmail), signatures, importance and receipts, tables and pictures in the editor, the full ribbon, contacts completion, `mailto:`, a draft in its own window, new-mail notices and badges, the door's "Waiting for you" and "Recent files", save all to Archive | — |
| 4 | If asked for | on-premises EWS, ManageSieve, JMAP, S/MIME, calendar invitations, import from Mail.app, mail on by default | a decision each |

---

## 11. Work packages

Order: **WP-0**, then four lanes side by side, then the fifth, then **N**. A lane is one lead with
its workers in one worktree (§12); the packages of a lane run in the order given.

| Lane | Packages | Starts |
|---|---|---|
| 1 Mail core | P2, then C, D, E | after WP-0 |
| 2 Mail data | A, B, and L's extractor (`codes.rs`, `codes.toml`) | after WP-0 |
| 3 Web and engine | P1, then F; M | after WP-0 |
| 4 Shell | G | after WP-0 |
| 5 Mail views | H, I, J, K, and L's offer (`code_offer.rs`) | once A, B, F and G are on `main` |

Lanes 1 and 2 share `crates/baa-mail/Cargo.toml` and `src/lib.rs`: each keeps its changes to them
small, lands often and rebases before every push.

| WP | Owner (CLAUDE.md) | Depends on | Files |
|---|---|---|---|
| 0 Contract | orchestrator | — | stubs across `baa-model`, `baa-web`, `baa-mail`, `baa-ui/src/mail/mod.rs`, root `Cargo.toml`, CLAUDE.md ownership |
| P1 Document proof | baa-web, engine (harness) | 0 | `baa-web/src/core_host/document.rs`, `baa-chromium/tests/mail-document.*` |
| P2 Stack proof | baa-mail | 0 | `baa-mail/Cargo.toml`, `src/backend/imap.rs` (connect, login, one fetch), `src/secrets.rs` |
| A Store | baa-mail | 0 | `src/store.rs`, `src/thread.rs` |
| B MIME | baa-mail | 0 | `src/mime.rs`, `src/compose.rs` |
| C IMAP sync | baa-mail | P2 | `src/backend/imap.rs`, `src/sync.rs` |
| D Send | baa-mail | P2 | `src/send.rs` |
| E Accounts, OAuth | baa-mail | P2 | `src/account.rs`, `src/provider.rs`, `src/oauth.rs`, `src/secrets.rs` |
| F Documents | baa-web | P1 | `baa-web/src/core_host/document.rs`, `lib.rs`; `baa-ui/src/mock.rs` |
| G Shell | baa-ui, baa-model | 0 | `browser.rs`, `sidebar.rs`, `library.rs`, `browser/{page_area,nav,actions,spaces}.rs`, `actions.rs`, keymap, icons; `baa-model/src/lib.rs` |
| H Nav and list | baa-ui (mail) | A, G | `mail/nav.rs`, `mail/list.rs` |
| I Reader | baa-ui (mail) | A, B, F, G | `mail/reader.rs`, `mail/commands.rs` |
| J Composer | baa-ui (mail) | B, D, F, G | `mail/compose.rs`, `mail/tabs.rs` |
| K Setup and settings | baa-ui (settings) | E | `mail/setup.rs`, `settings/mail.rs` |
| L Codes | baa-mail, baa-ui (mail) | A, B, M's contract | `baa-mail/src/codes.rs`, `codes.toml`; `baa-ui/src/mail/code_offer.rs` |
| M Engine | engine | 0 | `chromium/baa/codes.mm`, `baa.h`, `BUILD.gn`; `baa-web/src/core_host/{ffi,dispatch,page}.rs`; a harness test |
| N Wiring and docs | integration, CI | all | `crates/baa/src/main.rs`, `.github/security-lint.sh`, PRIVACY.md, README, PLAN.md |

**WP-0 Contract** (orchestrator; one commit touching contracts only)
- §8 as stubs: `Pane`, `Showing`, `MailSettings` with serde defaults; `create_document`,
  `set_document`, `fill_code`, `site_of`, `PageEvent::CodeField` with `MockHost` recording them and
  `CoreWebHost` doing nothing yet; `crates/baa-mail` with the types, `Backend`, `Secrets`, a `Fake`
  backend and in-memory secrets; `Browser.mail = None`.
- CLAUDE.md: the ownership lines for `crates/baa-mail` and `crates/baa-ui/src/mail`, and §9's rules
  once approved.
- Tests: a session saved by 0.1.9 (a split of two tabs) loads and saves unchanged; unknown
  `showing` loads as `Browser`; old settings load with mail off.
- Done when: `cargo check --workspace` and every existing test pass, and nothing the user sees has
  changed.

**WP-P1 Document proof** (baa-web; the harness test belongs to the engine owner)
- `document.rs`: `page_html(kind)` and `write_script(html)`, pure functions; `create_document` as
  `about:blank` plus the write.
- Tests (offline): the policy string for each kind; the sandbox attribute has no `allow-scripts`,
  `allow-same-origin`, `allow-forms` or `allow-top-navigation`; a body holding `</iframe>`,
  quotes, U+2028 and `</script>` stays inside the `srcdoc`; `set_document` on a page with a URL
  does nothing.
- Harness test `mail-document` as §6 "Proof". CI runs it; the agent does not.
- Done when: the harness run is green, or the report says which expectation failed and the fallback
  is costed.

**WP-P2 Stack proof** (baa-mail)
- Choose the IMAP library against: no async runtime, a release in the last 6 months, IDLE, UIDPLUS,
  MOVE, CONDSTORE, SASL XOAUTH2. Add it, `rustls` with the platform verifier, `mail-parser`,
  `mail-builder`, `security-framework`. List every new crate and its licence
  (`cargo metadata`) against `about.toml` in the report.
- Tests: a scripted server over an in-memory stream (greeting, CAPABILITY, LOGIN, LIST, SELECT, UID
  FETCH, IDLE and its EXISTS); a server that offers no TLS is refused; the Keychain `Secrets`
  compiles and the in-memory one round-trips.
- Done when: `cargo test -p baa-mail` passes and `cargo build -p baa` still builds.

**WP-A Store** (`baa-mail`)
- §4's tables, the version rule, `rows`/`count`/`conversation`/`search`, threading by `References`
  and `In-Reply-To` (and `X-GM-THRID` when given), unread and total counts kept by trigger or by
  query (`ponytail:` by query until a mailbox of 100k rows is slow).
- Tests: a newer file is refused untouched; an older one keeps drafts and view rows; "All inboxes"
  of two accounts orders by date and the scope filter narrows it; a reply joins its conversation; FTS
  finds a word of the body; a Space sees its accounts and the shared ones, never another Space's.

**WP-B MIME** (`baa-mail`)
- `Body { html, text, attachments, has_remote_images }` from raw bytes: the best alternative part,
  `cid:` images inlined as `data:`, plain text escaped, a snippet, the `Authentication-Results`
  verdict. `compose.rs`: a `Draft` to RFC 822 (multipart/alternative, attachments, `Message-ID`,
  `In-Reply-To`, `References`), reply and forward quoting, address-list parsing.
- Tests: a corpus in `tests/mail/` (nested multiparts, bad charsets, a 10 MB attachment, a header
  folded mid-word, a body that is only HTML); a parse of each never panics, also after random byte
  changes; a reply's headers thread; a file name with `../` comes out as its last component.

**WP-C IMAP sync** (`baa-mail`)
- The account thread: connect, folder list with special-use (and the Gmail preset's label folders),
  first sync of the newest 30 days of headers a folder, bodies on demand, then by `UIDNEXT`,
  `UIDVALIDITY` and CONDSTORE where offered; IDLE on INBOX; the `op` queue for flag, move, delete
  and append, replayed after a reconnect; back-off on failure; `Event`s.
- Tests, on the scripted server: a first sync fills the store; a new message during IDLE raises
  `Changed`; a changed `UIDVALIDITY` refills the folder; a move made offline is replayed once; a
  dropped connection reconnects with back-off and logs no host above debug.

**WP-D Send** (`baa-mail`)
- curl SMTP: implicit TLS or required STARTTLS, password or bearer on stdin (`-K -`), the message
  from a 0600 file in the mail directory that is removed afterwards; then append to Sent (not for
  Gmail, which files it itself); the outbox retries and reports.
- Tests: `smtp_args` holds `--ssl-reqd`, `--proto` and `--proto-redir` with mail schemes only and
  no secret; a failed send leaves the draft in the outbox and raises `Failed`.

**WP-E Accounts and OAuth** (`baa-mail`)
- `Account` rows and their Space; `provider.rs` presets (iCloud, Gmail, Fastmail, Yahoo, generic);
  `add_account` tests the sign-in before it stores anything; the Keychain `Secrets`;
  `remove_accounts_of`. `oauth.rs`: the authorize URL (PKCE, `state`), the loopback listener, the
  exchange and refresh by curl, tokens to `Secrets`; Google in this stage.
- Tests: an ephemeral Space is refused by every call; removing an account removes its secrets and
  rows; the listener refuses a wrong `state`, a second request and a non-loopback peer; a refresh
  that fails marks the account "sign in again" and keeps its mail; "basic authentication is off"
  from a server reads as that, not as a wrong password.

**WP-F Documents** (`baa-web`)
- P1's module finished for both kinds; remote images as a second policy, applied by a fresh
  document; `MockPage` keeps what was set, for the UI tests.
- Tests: P1's, plus the editor policy and a round trip of `set_document` then the read script.

**WP-G Shell** (`baa-ui`, `baa-model`)
- §5 in full, with `mail/mod.rs` supplying placeholder nav, view and door until wave 2.
- Tests (`tests_mail.rs`, on `MockHost` and the `Fake` backend): ⌘⇧M shows mail and comes back to
  the same tab; Space A in mail and Space B in the browser each return as left; a split of mail and
  a tab renders both and survives a save and load; the Mailbox rail item is absent in an ephemeral
  Space and a private window, and nothing is written there; a swipe up over the bottom bar opens the
  Archive in the third Space; deleting a Space names its accounts; with mail off nothing is drawn.

**WP-H Nav and list**, **WP-I Reader**, **WP-J Composer**, **WP-K Setup and settings** (`baa-ui`)
- Each builds its part of mocks A2/B2 and A3/B3 in both skins, from §2's table, on the `Mail` API.
- H tests: two accounts make one list and the chips filter it; the unread count follows a read;
  10,000 rows scroll through `uniform_list` without building them all.
- I tests: selecting a row sets the document with the body; remote images stay off until the
  banner's button, which makes a new document; a link's open request is adopted, and no test finds
  `load_url` with a URL from a message; delete, archive, move, flag and read reach `Mail::act`.
- J tests: a reply is a new tab with the quoted text and the right recipients; autosave writes a
  draft; send calls `Mail::send` and closes the tab; closing a changed draft asks first; the From
  field offers only the Space's accounts.
- K tests: the app-password path and the Google path each end in `add_account`; "every Space" is
  stored as no Space; removing an account asks first; a failed sign-in shows the server's reason.

**WP-L Codes** (`baa-mail`, `baa-ui`)
- `codes.rs` and `codes.toml` as §7 steps 4 and 5; `code_offer.rs` as steps 2, 3 and 6.
- Tests: the origin-bound line wins over a heuristic match; a 6-digit year or price is not a code;
  a sender of another site is not offered and an alias is; DMARC failure hides the code; the
  corpus under mutation never panics; the offer ignores a click in its first 500 ms and Enter
  always; nothing is offered in an ephemeral Space, in a background tab, or for a cross-site frame;
  two fields in 10 seconds cause one check.

**WP-M Engine** (engine orchestrator; `chromium-milestone` skill's patch rules apply to any patch)
- §7 "Engine". ABI 17 on both sides; a harness test with a local page (field focus reports, blur
  withdraws, fill lands, a cross-site frame reports its own origin).
- Checked with the engine syntax check; built by `chromium.yml` once, when the orchestrator asks.
- Done when: the release engine passes the harness test and `ci.yml`'s privacy job.

**WP-N Wiring and docs** (integration, CI)
- `main.rs` opens `Mail` in the data directory and hands it to `Browser` through `Shared`;
  `security-lint.sh` as §9; PRIVACY.md, README, PLAN.md §3 and §4.
- Done when: `cargo build -p baa`, `cargo test --workspace` and clippy pass, and §1's list is
  walked through by the user on a bundled build.

Stages 2 to 4 get their work packages when the stage before them ends; §15 is their scope.

---

## 12. How the agents work

Three levels (user, 2026-11-22):
- **The orchestrator** (the session that wrote this plan) owns the contracts (§8), starts the lanes,
  answers their questions, and does the final review of the whole change. It does WP-0 and WP-N.
- **A lead** (Opus 5.5, at most five at once) owns one lane of §11. It splits its packages into
  pieces with separate files, gives each piece to a worker, reviews every line a worker wrote
  before it is committed, and does itself whatever needs judgment (a security policy, a library
  choice, a contract question).
- **A worker** (Sonnet 5.5, at most three under a lead) writes the code and tests of one piece.

Rules for all of them:
- One worktree a lane. The lead's workers edit in that worktree, each in its own files; the lead
  runs the builds and makes the commits. So a lane compiles gpui once, not once a worker.
- A lane lands on `main` itself (CLAUDE.md "Git"): small commits, each passing `cargo test` for the
  crates it touches, fast-forward only.
- A lane edits only its packages' files. A contract that does not fit is reported to the
  orchestrator, not changed.
- Nobody runs the app, the harness or a GUI check. Proof is `cargo test`, clippy and the build.
- A lead ends each package with the quality review and the comment pass on the package's changes,
  then reports: commits, tests added, dependencies added, shortcuts taken (`ponytail:`), what is
  left open. It deletes its build output; the orchestrator removes the worktree.
- The orchestrator reads every landed diff, runs the workspace tests when a lane ends, and writes
  the stage's outcome into this file.

---

## 13. Risks

| Risk | Cheapest test | Fallback |
|---|---|---|
| A document cannot be written into `about:blank`, or the policy does not hold | WP-P1's harness test | a `baa://mail` data source in the ABI 17 batch |
| The IMAP library does not fit blocking threads, or a licence is not accepted | WP-P2 | `imap` 2.4; or `about.toml` gains the licence with the user's say |
| Google does not verify the restricted scope for a public release | ask Google early; the test-user mode carries development | Gmail by app password; or the Gmail API with narrower scopes |
| A Microsoft 365 tenant refuses the app | try on a real tenant in stage 2 | publisher verification; the tenant's admin consents; IMAP where the tenant allows it |
| Chromium does not classify code fields with autofill off | WP-M's first reading | a focused-element check for `autocomplete="one-time-code"` only |
| The engine build takes days | it is one build, started as soon as WP-M passes the syntax check | codes ship one release after the rest of the spike |
| `Pane` breaks a saved session | WP-0's round-trip test on a 0.1.9 session | keep `tabs` and add a separate `mail` flag to `Split` |
| A swipe up over the bottom bar collides with something | WP-G's test; the user tries it | a button only, and the existing swipe past the first Space |
| A mail server on a port other than the six | none needed: it is refused with a message that names the rule | the user widens the rule |
| Dev builds ask for the Keychain on every rebuild (unsigned) | the first run of WP-P2's code | a file-backed `Secrets` for `baa Dev` only, never in a release |
| Mail HTML written for a white page looks wrong in a dark Space | look at ten real messages | the message card stays paper white, as mock B2 has it |

---

## 14. Decisions the user took, and what is open

Answered on 2026-11-22:
1. Views in `baa-ui`, everything else in `baa-mail`.
2. Mail protocols outside Chromium are allowed, on their standard ports only (§9).
3. The engine build for codes (ABI 17) is part of the first spike.
4. The gesture is a two-finger swipe up over the bottom bar, in any Space; the mail nav also lists
   the other Spaces' mailboxes.
5. Inside mail ⌘N is New Message and ⌘R is Reply.
6. The OAuth code is built as if the client ids exist. The user registers the clients outside this
   work and gives the ids later: a Google Cloud client (type "Desktop", scope
   `https://mail.google.com/`), and an Entra app (any organisation and personal accounts, public
   client, loopback redirect).
7. Remote images: ask a message, with "always for this sender".
8. Exchange on-premises stays optional, in stage 4.

Open, and not needed before stage 2:
- **Proton Mail Bridge** listens on 127.0.0.1 at ports 1143 and 1025, which answer 2 does not
  allow. Either loopback gets its own exception, or Proton is not supported.

---

## 15. Outlook's features without a service

What each needs, since baa has no server of its own.

| Feature | How | Limit | Stage |
|---|---|---|---|
| Read, flag, answered | IMAP flags; Graph properties | — | 1 |
| Conversations | headers; Gmail and Graph give ids | — | 1 |
| Combined inbox, scope filter | a query over the Space's accounts | — | 1 |
| Verification codes | §7 | the message must arrive while baa runs | 1 |
| Shared mailboxes | Graph, added by address; IMAP shared namespaces show as folders | Graph cannot list the ones a user may open | 2 |
| Send as, on behalf | Graph `from`; an IMAP account's extra identities | the server must allow it | 2 |
| Categories | Graph categories; Gmail labels; IMAP keywords | a server without keywords keeps them local | 3 |
| Follow-up with a date | Graph flag dates; elsewhere `local` | local dates do not reach other clients | 3 |
| Pin, snooze | `local` | other clients do not see them | 3 |
| Scheduled send, delay delivery | Graph defers on the server; elsewhere the outbox | elsewhere baa must be running at the time | 3 |
| Focused / Other | Graph's own; elsewhere bulk-mail headers | a heuristic outside Graph | 3 |
| Search folders | saved queries on `mail.db` | only mail that has synced | 3 |
| Sweep, quick steps | local sequences of `Action`s | — | 3 |
| Rules | a local engine at sync time; server rules on Graph and Gmail filters | local rules run only while baa runs | 3 |
| Signatures | `local`, one an account | — | 3 |
| Importance, read receipts | headers | the recipient's client decides | 3 |
| Contacts completion | addresses seen in mail | no directory lookup until Graph People | 3 |
| Groups, Loop, Draw, Translate, Encrypt | — | not planned | — |
