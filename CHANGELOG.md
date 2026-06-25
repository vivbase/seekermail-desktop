# Changelog

All notable changes to the SeekerMail code repo are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
uses [Conventional Commits](https://www.conventionalcommits.org/).

> **⚠️ Pre-release — early-access public preview, not the commercial v1.0 GA.** The app
> manifest version is **`0.6.0`** (`package.json` + `src-tauri/Cargo.toml` +
> `src-tauri/tauri.conf.json`), shipped under the milestone tag **`v0.6.0-beta`** in the
> scheme `v0.1 Internal → … → v0.6 Beta → v0.7 RC → v1.0 GA`. Pre-1.0 builds are openly
> downloadable previews, published as GitHub **pre-releases** (never the _Latest_
> release), and carry **no commercial promise** — no pricing, SLA, or warranty; those
> begin at **v1.0 GA**. The **v1.0.0 / v1.0.1** sections below describe _planned_ GA
> scope and have **not** been released. On each release the top `[Unreleased]` section is
> renamed to the version + date and a git tag is pushed (`--prerelease` for suffixed
> tags; the public Latest release only for a no-suffix GA tag).

## [Unreleased]

### Fixed

- **fix(release): production builds now ship the REAL runtimes (real network + real
  semantic embeddings).** The macOS release leg (`.github/workflows/release.yml`) ran a
  bare `pnpm tauri build` with **no `--features`** and never fetched the embedding model,
  so signed/notarized DMGs silently shipped the **stubbed transport _and_ the
  deterministic offline (bag-of-words) embedder** — no real IMAP/SMTP and no real bge-m3
  semantic search — even though the inside-out codesign step already signed the ONNX
  Runtime dylib (proving the real runtimes were intended; the flags had simply been
  dropped). The macOS leg now fetches + checksum-verifies the bge-m3 weights through a
  lock-cached `pnpm run setup:model` (bundled via `tauri.conf.json` `resources/*`) and
  builds with `--features live-net,local-embed`. Without this, every shipped build
  defeated the local-first AI value proposition — semantic search and AI reply context
  ran on a hash, not a model.

### Added

- **ci(feature-build): Windows compile guard for the OFF-by-default real runtimes.** A
  new `feature-build-windows` job compiles `--features live-net` and
  `--features live-net,local-embed` on `windows-latest`, exercising the
  `ort`/onnxruntime-sys link path on MSVC (the 2.2 GB model is not needed to compile).
  The Windows release leg currently ships `--features live-net` (real IMAP/SMTP) only;
  Windows `local-embed` stays off until this guard is reliably green, at which point the
  release leg flips to `live-net,local-embed` and gains a cached `setup:model` step like
  macOS.

## [0.6.0-beta] - 2026-06-24

Wires the previously-stubbed mail pipeline end-to-end: the app now **fetches real
IMAP mail and renders it**. Before this, the default build shipped offline transports
_and_ the `live-net` build's IMAP session was a stub _and_ the mail-list read commands
were never implemented — so the inbox was always empty (`sync_state.last_sync_result =
network_error`, 0 mails) even with a configured account.

### Added

- **feat(i18n): ship all 21 locales — the UI is now fully localized (10 §2).** Previously only
  `en` resources existed, so every non-English locale fell back to English (the "English-only,
  1 of 21" gap from knowledge base `docs/analysis/37`). This adds `src/i18n/resources/<locale>/`
  for the other **20 locales** (`zh-CN zh-TW ja ko vi ar es pt fr de he hi bn ur it nl pl ru tr sv`),
  each mirroring all **20 namespaces** = **1,261 keys/locale** (25,220 translated strings). `index.ts`
  now assembles the resource tree from **`import.meta.glob('./resources/*/*.json', { eager: true })`**
  instead of hard-coding `en`, so the app bundles every locale (offline / local-first — no network
  backend, consistent with the unused `i18next-http-backend` seam) and adding a locale is just dropping
  its folder in. Switching language now renders translated copy and flips `dir`/`script` (RTL for
  `ar`/`he`/`ur`) via the existing `applyLocale`. ICU MessageFormat plural branches use each language's
  CLDR categories (e.g. `one/few/many/other` for `ru`/`pl`; `zero…other` for `ar`; `other`-only for
  `zh`/`ja`/`ko`/`vi`), `{…}` placeholders and product names (SeekerMail, GTE, Agent-IM, AI, …) are
  preserved, and every locale was validated for key-parity, placeholder-parity and ICU well-formedness
  against `en` (`tsc --noEmit` + the i18n vitest suite pass). Note: these are high-quality machine
  translations that still need **native-speaker QA** before each locale is marked release-ready per
  `docs/dev/10 §9`. Closes ENG-02 (`docs/analysis/38`).
- **feat(settings): surface the global AI master switch (F5 §4.5).** Settings → AI Providers
  gains an **AI Master Switch** that disables every AI capability across all accounts for
  **24 hours / 48 hours / indefinitely**, or resumes it immediately. It drives the dedicated
  `set_ai_disabled` command (previously registered in `lib.rs` but never surfaced) through the new
  `useSetAiDisabled` hook, reading current state from `get_setting("ai.disable_until")`; the F5
  fallback router already honors that deadline for every AI call. Closes the `set_ai_disabled`
  half of the front/back contract gap. (Knowledge base `docs/analysis/30_*` §5.)
- **feat(sync): IMAP IDLE push — new mail now arrives in seconds, not minutes.** Each active
  account keeps one long-lived connection parked in IMAP IDLE (`src/imap/idle_task.rs`); the
  instant the server reports a mailbox change it pokes the existing poll task's `Notify` — the
  same wake the manual sync uses — so every fetch still runs in one place and the two paths can
  never double-fetch a UID. On the RFC-2177 ~29-min keepalive it re-issues IDLE; on a dropped
  connection it reconnects with capped backoff; on an auth error or shutdown it exits. The
  interval poll stays as the safety-net and the fallback for servers without IDLE. **No backend
  service is required** — the IDLE connection is held by the local client straight to the mail
  provider, so the local-first / data-sovereignty model is unchanged. New `ImapSession::idle_wait`
  transport seam (live adapter over `async-imap` 0.9.7 + scriptable test fake), spawned by the
  scheduler alongside each poll task. Real push requires `--features live-net`.
- **feat(ui): "Get Mail" pinwheel in the nav rail.** An icon-only refresh control
  (`src/components/layout/GetMailButton.tsx`) sits at the top of the sidebar — no permanent text;
  the "Get Mail" label shows only as a hover tooltip. Clicking force-syncs every active account via
  `trigger_sync`; the pinwheel spins **in place (non-blocking — it never covers the UI)** until the
  `sync:complete` / `sync:error` event lands.
- **feat(mail): remote images load privately through the backend, with a prominent one-click bar.**
  Remote images stay blocked by default, but loading them no longer lets the WebView connect to the
  sender's server: the new `fetch_remote_image` command fetches locally from Rust with **no cookies, no
  Referer, and no User-Agent**, accepts only an `image/*` response under a 16 MB cap, caps + re-validates
  redirects, and refuses internal/localhost hosts (basic SSRF guard); the bytes return as a `data:` URI
  (CSP already allows `img-src data:`). The reading-view bar (`RemoteImageBar`) is now a clear amber
  banner with a privacy note and one-click **Load images** / **Always from this sender**, and allow-listed
  senders now auto-reveal on reopen (previously they stayed blocked). New `useFetchRemoteImage` hook +
  `lib/mailImages.ts` helpers + `RemoteImage` DTO. A local fetch can't hide the device IP (proxy/VPN is
  out of scope to stay local-first) but removes the other correlating headers. (Knowledge base
  `docs/analysis/47`.)

### Changed

- **fix(ai): the first-run AI-activation prompt no longer offers Full Auto, and Semi-Auto / Manual Only are now actually selectable.**
  The "Start in Semi-Auto" step (`src/components/ai/AiActivationPrompt.tsx`) previously rendered the three reply tiers as
  inert, non-clickable rows and hard-coded activation to Semi-Auto — so Full Auto and Manual Only looked like choices but
  ignored clicks. The step is now a real radio group offering only **Semi-Auto (recommended, default)** and **Manual Only**;
  the activate button follows the selection and writes the picked `authLevel` (2 / 1). **Full Auto is withheld at first run**
  because it is already locked until an account has ≥ 50 approved drafts (F_E3 §4.1, `AuthLevelSection`), so day-one users
  could never have applied it anyway; it stays available, lock-gated, under Settings → AI Reply Mode. i18n keys `mode_full*`
  removed and `cta_activate_semi` → interpolated `cta_activate` in `en` + `zh-CN`. Knowledge base: `docs/analysis/30`,
  `docs/product/AI_MODES_DESIGN.md` §7.4.
- **fix(team): the sidebar TEAM badge now tracks real read/unread state instead of a static-looking count.**
  The TEAM nav badge previously showed `count_pending_queries` — the number of open _decision_ cards —
  so reading the channel never changed it; and the AGENTS nav badge simply showed the _number of
  configured agents_, a red count that never cleared. Neither matched what a red badge implies
  ("things you haven't seen"). The TEAM badge is now a **hybrid** driven by the new `count_team_unread`
  command / `useTeamUnreadCount` hook: it counts unread agent messages **plus** unresolved decision
  cards (`status='pending' OR (sender_type='agent' AND read_at IS NULL)`). Opening the TEAM channel calls
  the new `mark_im_channel_read` command (via `useMarkTeamRead`), which clears the unread half while open
  decisions keep counting until answered/skipped. This finally wires up the previously dead read/unread
  plumbing (`im_messages.read_at`, `mark_read`). `count_pending_queries` is kept for the Dashboard
  pending tile. New Rust tests cover `count_unread`/`mark_all_read`; updated vitest covers the new badge
  hook and read-on-open. (Knowledge base `docs/analysis/36_*`.)
- **change(sync): default poll interval lowered 300s → 60s.** With IMAP IDLE now delivering new
  mail in near-real-time, the interval poll is only a safety net plus the fallback for servers
  without IDLE, so a tighter 60s floor is cheap and keeps that fallback responsive. New accounts
  get 60s from `AccountRepo::create`; migration `017_default_sync_interval.sql` brings existing
  accounts still on the old 300s default down to 60s (user-customised values are left untouched).

### Removed

- **The AGENTS sidebar nav badge.** It only ever rendered the agent count (always ≥1 once you have an
  agent), so as a red badge it read as permanent noise rather than a notification. The Agents roster page
  still shows live active/idle/paused/query counts; the rail item is now badge-free. (Knowledge base
  `docs/analysis/36_*`.)

### Fixed

- **fix(mail): inline (`cid:`) images now render in the reading view.** Company logos, signature
  images and in-body pictures never showed — the sanitiser correctly preserved `<img src="cid:...">`,
  but the inline part's bytes were never downloaded (auto-download excluded them with `is_inline = 0`)
  and nothing resolved `cid:` to bytes for the WebView, which cannot load a `cid:` URL, so it rendered
  blank/broken frames. The new `get_inline_images` command fetches the inline bytes on demand (reusing
  the attachment-download transport) and returns them base64; the reading view (`SanitizedMail`) swaps
  each `cid:` image to a `data:` URI after the DOMPurify pass, with unresolved ones hidden via CSS to
  avoid a broken-image flash. Inline images ship inside the message (no network, no privacy cost), so
  they load automatically — no "load images" gate. New `useInlineImages` hook + `InlineImage` DTO; the
  remote-image blocker is unchanged. (Knowledge base `docs/analysis/47`.)
- **fix(sync): a newly added mailbox imports its existing mail again.** Adding an account had
  stopped importing any history. By design (F*A4 §6) the first incremental poll of a brand-new
  account seeds the UID cursor at the mailbox's current high-water mark and fetches nothing itself
  — existing mail is the history backfill's job — but the first sync never actually \_started* that
  backfill. The only place a backfill was ever kicked off is `set_knowledge_depth` (the wizard's
  last step), so any add path that didn't reach it (and any single backfill that errored or raced)
  left the inbox empty. The previous behavior masked this because the first poll used to fetch
  everything from `UID 1`; once that was changed to the baseline-only seed, the backfill became the
  _sole_ importer and the missing trigger surfaced as "mailbox added → no mail shows". The first
  sync now starts the backfill itself (`imap/sync.rs`, step 3a), so every add path — wizard,
  onboarding, re-enable, OAuth, programmatic — imports history; a new per-account in-flight guard in
  `imap/backfill.rs` collapses the first-sync trigger and the wizard's own `set_knowledge_depth`
  trigger into a single run so they can't race on `backfill_state`. Regression coverage added:
  `imap::backfill` previously had **no tests** — added one proving the backfill imports existing
  history to the ingest channel, plus `imap::sync::first_sync_imports_existing_mail_for_new_account`
  proving a freshly added mailbox imports its mail on the first sync (it fails without the trigger).
  Note: until a knowledge depth is chosen the initial fill imports all history (this matches the
  pre-regression behavior); a depth-bounded initial fill is a follow-up.
- **fix(mail): open links in emails with the system browser instead of hijacking the app window.**
  Email bodies are injected via `dangerouslySetInnerHTML` (`SanitizedMail`), and `<a href>` clicks had
  no handling — so clicking a link (e.g. a Google OAuth notice) navigated the Tauri webview itself,
  replacing the entire SPA with the destination page and leaving no way back. A new capture-phase
  interceptor (`lib/externalLinks.ts`, installed once in `main.tsx`) cancels the navigation for
  cross-origin web links and `mailto:`/`tel:` links and routes them to the OS default browser / mail
  client via the new `open_external_url` command (`commands/shell.rs`); same-origin routes and
  `#fragment` anchors still fall through to React Router. The backend re-validates the scheme as the
  trust boundary — only `http`/`https`/`mailto`/`tel` reach the OS opener, and arguments are passed to
  the opener process directly (no shell), so `javascript:`/`data:`/`file:` and injection are refused.
  New Rust tests cover the scheme allowlist; new vitest tests cover external-vs-internal link routing.

- **fix(attachment): address attachment bytes by a real MIME part index (not the row UUID).**
  The deferred download passed the attachment's database UUID as the IMAP part specifier
  (`FETCH BODY.PEEK[<uuid>]`), which is never valid — so attachment download could not locate
  the bytes against a real server. The parser now records each attachment's 0-based index within
  the message's `attachments()` iterator (new column `attachments.part_index`, migration `016`),
  and the live IMAP session fetches the full message and slices out that part by index — correct
  for any MIME nesting, without re-deriving fragile `BODYSTRUCTURE` part numbers. The parser also
  now stores the **real `content_type`** (e.g. `application/pdf`) instead of hardcoding
  `application/octet-stream`, so extraction routing no longer depends on the filename extension
  alone. New tests in `imap/parser.rs` lock in the content-type/part-index extraction and the
  exact byte-slicing the download relies on. (Knowledge base `docs/analysis/30_*` G2.)

- **fix(send): the offline build refuses to send instead of silently succeeding.**
  With `live-net` off, `transport_send` logged "message accepted by offline transport" and returned
  `Ok(())` — so a feature-less `cargo build` produced a client that _appeared_ to send while nothing
  left the machine. A real feature-less binary now returns `SMTP_SEND_FAILED` ("nothing was
  transmitted; rebuild with --features live-net"); the in-crate test stub still accepts (no network)
  so the cancel window, SENT persistence, and `mail:new` event stay exercisable in unit tests. The
  shipped product is unaffected (it is always built with `--features live-net`). (Knowledge base
  `docs/analysis/30_*` G1.)

### Docs

- **docs(readme): backfill the code-repo `README.md` to match reality.** The status line no longer
  claims "product code has not started yet" (90k+ lines exist; version `0.1.0`); the tech-stack
  table describes the GTE index accurately (local cosine snapshot today; LanceDB-class ANN as the
  scaling target) instead of asserting LanceDB; the Build & Run section lists the real `pnpm`/`cargo`
  workflow and the `--features live-net` requirement. A "NOT RELEASED — forward-looking draft" banner
  was added to `docs/releases/v1.0.0_ga_release_notes.md` (and its stale "Deals" section, dropped by
  migration `014`, is flagged). The stale `net/live.rs` "remaining binding point" header was corrected.

### Changed

- **change(account): Gmail mailbox import moves to IMAP + App Password; mailbox OAuth is now
  Microsoft/Outlook only** (knowledge base `../seekermail-desktop-2026/docs/analysis/29_*`). Google's
  `https://mail.google.com/` is a _restricted_ OAuth scope (annual paid CASA security assessment to
  ship publicly), so Gmail no longer uses OAuth here — users add Gmail via **IMAP / SMTP + a Google
  App Password** (autodiscover fills `imap.gmail.com` / `smtp.gmail.com`). `Provider::is_oauth()` is
  now `Outlook`-only; `account/oauth.rs` drops the Gmail authorize/token endpoints + client-id; the
  Add-Account wizard's OAuth protocol option is relabeled **"Microsoft / Outlook"** and the OAuth
  authorize step shows only for Outlook. "Sign in with Google" remains **identity-only** (SeekerMail
  ID, scopes `openid email profile`) and is unaffected — so scaling carries no Google restricted-scope
  assessment. PKCE primitives (`new_pkce`) were extracted to a neutral `account/pkce.rs` shared by the
  identity, recommended-provider, and Outlook flows. Follow-up (analysis/29 §7): migrate the Outlook
  redirect from the `seekermail://` custom scheme to a `127.0.0.1` loopback listener (mirroring the
  identity flow) for robustness + dev-testability.

- **fix(account): mailbox OAuth callback `code` is now percent-decoded (M17)** — `parse_mail_callback`
  url-decodes both `code` and `state` (matching the identity loopback path). Microsoft authorization
  codes commonly contain `/` (arriving `%2F`); undecoded, the token endpoint received a
  double-encoded `%252F` and the exchange failed. (Knowledge base
  `../seekermail-desktop-2026/docs/analysis/28` M17.)

### Added

- **feat(compose): rich-text formatting bar.** The Reply / Compose body is now a
  formatting-capable `contentEditable` editor instead of a plain-text box — bold,
  italic, underline, strikethrough, font size, text colour, highlight colour,
  bulleted & numbered lists, indent, alignment, block quote, insert link, clear
  formatting, and undo/redo. Outgoing mail is sent as a `multipart/alternative`
  message (a `text/html` part plus a plain-text fallback) and drafts persist their
  formatting; the SMTP transport already supported HTML bodies, so this change is
  frontend-only. New `src/components/compose/ComposeFormatBar.tsx` +
  `src/lib/richText.ts`; `ComposeEditor` became a `contentEditable` surface that
  mirrors plain text into the store for validation and quote seeding.
- **feat(identity): decouple SeekerMail ID from mailboxes (A6 rewrite)** — the SeekerMail ID is
  now an **independent, optional identity** created by signing in with Google, with **no link to
  imported mailboxes** (the "binding mailbox" model is retired). Signing out of the SeekerMail ID
  (`sign_out_seekermail`) now **clears only the local identity** — mailboxes and local mail are
  untouched — and removing a mailbox (`delete_account`) is allowed even for the **last** one (zero
  mailboxes is a valid state; the old "can't remove your only account" dead-end is gone). Adds
  migration `015_seekermail_id.sql` (single-row `seekermail_id` table, **no** `is_id_binding` on
  `accounts`), `storage/identity_repo.rs`, `commands/identity.rs` (`get_seekermail_id`, redefined
  `sign_out_seekermail`, `set_marketing_consent`, and stubbed `begin/complete_google_signin`
  pending the cloud backend — T121), and the `SeekerMailId` type. Frontend: rewritten
  `ipc/queries/identity.ts`, an independent `SeekerMailIdCard` (sign in / sign out + marketing
  opt-in), and `AccountList`/`AccountRow` with the binding UI removed. A consented, **opt-in
  marketing-contact email** (default OFF) is captured at the identity layer; mail content,
  contacts, and other mailbox addresses are never used. Specs:
  `../seekermail-desktop-2026/docs/function list/F_A6_seekermail_id.md` (rewritten) and
  `../seekermail-desktop-2026/docs/analysis/26_identity_decoupling_and_email_marketing_foundation.md`.
  Run `pnpm gen:types` to regenerate `bindings.ts`.

- **feat(settings): adjustable UI text size (Appearance → Text Size)** (analysis 25). A new
  Appearance control scales the whole interface proportionally through a single `--ui-scale`
  CSS variable (`#root { zoom: var(--ui-scale) }`) — five steps from Small (90%) to Largest
  (150%). Because every element scales together, the layout cannot break — important because
  the codebase is px-heavy, so text-only resizing would have overflowed fixed-height rows. The
  preference persists to `app_settings.ui.font_scale` and is injected as
  `window.__INITIAL_FONT_SCALE__` before first paint (FOUC guard), mirroring the dark-theme
  mechanism. Adds `src/lib/fontScale.ts`, `src/font-scale-boot.ts`, and Rust `initial_font_scale`;
  wired into the settings query hooks, the Appearance page, and the `en` settings locale.
  Keyboard shortcuts `Cmd/Ctrl +` / `-` / `0` step and reset the scale app-wide
  (`useFontScaleShortcuts`, mounted in `AppShell`). A second, independent **Reading text size**
  layer (`ui.reading_font_scale`, `--reading-scale`) scales only the sanitised email body —
  adjustable from Appearance and from an inline A−/A+ stepper in the reading view
  (`ReadingSizeControl`); adds `src/lib/readingScale.ts`.
- **feat(agent): Team channel AI replies (F_I5)** (`src-tauri/src/ai/team_chat.rs`). Agents
  now answer in the Agent-IM TEAM channel. When a human posts a text message,
  `post_im_message` spawns a detached task that picks a responder (`@DisplayName` mention →
  that agent, else the primary account), runs a local GTE semantic search over the agent's
  mailbox, packs any relevant hits into the prompt, calls the account's BYO provider
  (`Capability::Summarize` routing), and posts the reply back to the channel. With no
  relevant mail it answers as a general assistant; with no provider configured it posts a
  helpful "connect an AI model" note instead of staying silent. The composer gains a
  "replying…" indicator and faster polling while awaiting the answer (`TeamChannel.tsx`,
  `ChannelInput.tsx`, `team.json`). Before this, a Team message got **no response** —
  `post_im_message` only stored it and nothing triggered an agent reply.
- **feat(net): real `LiveImapFactory` streaming session** (`src-tauri/src/net/live.rs`).
  Implements `ImapSession` over `async-imap` 0.9 — TLS connect + `LOGIN`, `SELECT INBOX`,
  `UID SEARCH` (incremental `UID n:*` and `SINCE`), and `UID FETCH BODY.PEEK[]` —
  replacing the stub that returned "live IMAP streaming session not yet wired". A `NO`/
  `BAD` login response maps to `AuthInvalidCredentials` so the poll loop stops on bad
  credentials instead of backing off forever.
- **feat(mail): mail-list read backend.** New Tauri commands `list_threads`, `list_mails`,
  `get_mail`, `set_mail_read`, `set_mail_starred`, `archive_mail`, `delete_mail`
  (`commands/mail.rs`, registered in `lib.rs`), backed by paginated queries in
  `storage/mail_repo.rs`, plus `Thread`/`MailDetail`/`ListThreadsParams`/`ListMailsParams`
  in `types.rs`. The webview already called `list_threads`/`list_mails`/`get_mail`, but no
  Rust command existed, so the calls rejected in the packaged app and every mail surface
  rendered empty. All Mail / Unread / reading view now return real rows.
- **chore(scripts): `pnpm tauri:dev` / `pnpm tauri:build`** pass `--features live-net`, so
  shipped builds include the network transports. The Cargo default stays offline to keep
  unit tests fast and preserve the egress-compliance contract.
- **feat(ai): cloud-provider model picker + `list_cloud_models` command.** The Add Cloud
  Provider sheet's free-text **Model** field is now a dropdown: a curated per-vendor
  shortlist, a **Load models** button that pulls the live catalog from the provider
  (`GET /v1/models` via the new `list_cloud_models` Tauri command, backed by
  `openai::list_models` / `AnthropicClient::list_models`), and a **Custom** option for any
  model id. Picking a real model — rather than mistyping one (e.g. `GPT5.5` instead of the
  real id `gpt-5.5`) — is what unblocks the connection test and lets a key be saved. New
  `ListCloudModelsParams` DTO and `useListCloudModels` hook; the step-4 model field is now
  a read-only confirmation of the tested model.
- **feat(ai): one-click presets for the top global model providers.** The cloud-provider
  picker now offers OpenAI, Anthropic, Google Gemini, xAI Grok, DeepSeek, Alibaba Qwen,
  Mistral, Moonshot Kimi, Zhipu GLM and Meta Llama (plus Azure OpenAI and a generic
  OpenAI-compatible option). Choosing a preset prefills the vendor's API base URL and a
  curated current-model shortlist; everything else (live `Load models`, the connection
  test, and reply generation) flows through the existing OpenAI/Anthropic adapters. Defined
  by `CLOUD_PRESETS` in `AddCloudProviderSheet.tsx`.
- **fix(ai): version-tolerant endpoint URLs for OpenAI-compatible vendors.** The OpenAI
  adapter built `{base}/v1/chat/completions` and `{base}/v1/models` unconditionally, which
  doubled the version for vendors whose base URL already carries one — Gemini
  (`/v1beta/openai`), Qwen (`/compatible-mode/v1`), Zhipu (`/api/paas/v4`), xAI (`/v1`).
  `openai_endpoint` now appends the resource directly when the base path already has a
  `/v<digit>` segment and inserts `/v1` only for plain hosts (`https://api.openai.com`), so
  every preset's documented base URL resolves correctly.

### Changed

- **refactor(i18n): rename the "Authorization Level" / "Global Mode" UI label to "AI Reply Mode".**
  The per-account reply-mode control (Manual Only / Semi-Auto / Full Auto) already lives where the
  spec puts it — the `AuthLevelSection` under **Settings → AI**, mirrored by the Agents-page roster
  and `RoleEditor` — but it surfaced as "Authorization Level(s)" / "Global Mode", which didn't read
  as the semi-/full-auto _reply_ feature users were looking for. Renamed user-facing strings only
  (`en/agents.json` `agents_global_mode`, `agents_auth_level_label`, `agents_page_subtitle`;
  `en/aiDrafts.json` `auth_level_section_title`, `auth_level_section_desc`; `en/common.json`
  `agents_desc`). Code identifiers (`authLevel`, `AuthLevelSection`), IPC, and the Rust backend are
  unchanged, and the tier values (Manual / Semi-Auto / Full Auto) are unchanged. Brings the app in
  line with the prototype/spec reconciliation recorded in
  `../seekermail-desktop-2026/docs/analysis/23`.

### Fixed

- **fix(account): OAuth mailboxes can finish authorizing and import mail** (`account/oauth.rs`,
  `commands/accounts.rs`, `lib.rs`, `routes/settings/accounts/AddAccountWizard.tsx`). The
  Gmail/Outlook grant had no completion path: nothing caught the `seekermail://oauth/callback`
  redirect and the Add-Account wizard advanced without ever calling `complete_oauth_flow`, so the
  token never reached the Keychain and the account sat at `RE-AUTHORIZE` / 0 MB — and a
  credential-less account could be "created" from just an email. Added a `tauri-plugin-deep-link`
  handler (routes `/oauth/callback` → `oauth:mail_callback`, `/oauth/recommended` →
  `oauth:callback`); `begin_oauth_flow` now returns the CSRF `state`; the wizard gained an
  authorize step (deep-link auto-complete + manual code-paste fallback) that blocks until the grant
  completes; and `complete_oauth_flow` clears the auth-error and re-arms the account's poll so mail
  imports immediately. Half-authorized accounts are cleaned up on cancel. NOTE: the deep-link
  auto-callback needs a packaged build + a real `SEEKERMAIL_GOOGLE_CLIENT_ID`/
  `SEEKERMAIL_MICROSOFT_CLIENT_ID` to verify end-to-end.
- **fix(account): "Sign out of SeekerMail" now actually signs out** (`src-tauri/src/account/mod.rs`,
  `src-tauri/src/commands/accounts.rs`, `src/ipc/queries/identity.ts`,
  `src/components/account/SeekerMailIdCard.tsx`). The full sign-out looped `delete_account` over
  every mailbox, but that command refuses to remove the **last** account — so the binding mailbox
  never deleted, the app never reached zero accounts, and the user was stuck signed in. Added a
  dedicated `sign_out_seekermail` command + `AccountService::sign_out` that disconnects every
  mailbox (ordinary first, binding last), bypassing the last-account guard that still protects the
  per-mailbox delete. The confirm dialog now closes once the call settles (not before) and surfaces
  an error if it fails, instead of closing instantly over a silent failure.
- **fix(ai): connection test now verifies current reasoning models (e.g. `gpt-5.5`).**
  `verify_ai_provider`'s OpenAI/Anthropic probe issued a one-token chat completion carrying
  `max_tokens` + `temperature`, which current reasoning models reject (they require
  `max_completion_tokens` and the default temperature) — so even a correct model id failed
  the test with a 4xx. The probe now verifies by reading the model catalog
  (`GET /v1/models`) and confirming the chosen id is present: no chat-only parameters, no
  token spend. Minimal OpenAI-compatible gateways without a `/v1/models` route fall back to
  the original chat probe, so they still verify; `Auth` / `RateLimited` / `Unreachable`
  stay conclusive.
- **fix(ai): reply generation works with reasoning models on every endpoint.** The OpenAI
  chat path (`chat` / `chat_stream`, which also serves Azure / Gemini / OpenAI-compatible
  gateways) sent `max_tokens` + `temperature` unconditionally — rejected by current
  reasoning models (`gpt-5.x`, the o-series), which require `max_completion_tokens` and the
  default temperature. The request shape is now chosen per model and **adapts on a 400**:
  when the endpoint reports an incompatible parameter, it renames `max_tokens` ↔
  `max_completion_tokens` and/or drops `temperature`, then retries (bounded by
  `MAX_PARAM_RETRIES`). So a reasoning model on real OpenAI and a legacy gateway that only
  accepts `max_tokens` both succeed from one code path. The 400 body is inspected solely to
  choose the adjustment and never enters an error payload or a log line (09 §5). Anthropic
  (Messages API) and local Ollama already used their correct parameters and are unchanged.
- **fix(mail): HTML email no longer renders as a grid of empty boxes.** `SanitizedMail`
  forced a `1px` border on every `<td>`/`<th>`, so marketing mail — which nests tables
  purely for layout — drew a box around every spacer / image cell (and blocked remote
  images showed as empty frames). Removed the blanket cell border, hide blocked images
  until loaded, and contain wide tables. Both sanitiser passes now also preserve inert
  presentational attributes (`align`, `valign`, `bgcolor`, `width`, `height`,
  `cellpadding`, `cellspacing`, `border`) and a **CSS-scrubbed** `style` attribute, so
  HTML mail keeps its intended columns, spacing, and colour. Inline CSS is filtered to a
  safe property allowlist (`sanitize::scrub_style` ↔ `lib/cssScrub.ts`) that drops
  `url()`, `position`, `expression()`, `@import`, and `javascript:`, preserving the
  two-pass security model (T027/T028, 07 §10). New unit tests cover both passes.
- **fix(events): live mail refresh.** `sync:complete` / `mail:new` invalidated
  `["threads", accountId]`, but the all-accounts list keys on `["threads","all"]` and the
  flat list on `["mails", …]` — neither matched, and `["mails"]` was never invalidated.
  Both now invalidate the `["threads"]` / `["mails"]` prefixes, so fetched mail appears
  without a manual view switch.

### Verified

- macOS / Apple Silicon: `cargo check --features live-net` green; the debug bundle
  launches, syncs a Gmail account over IMAP (27 messages, 23 threads,
  `sync_state.last_sync_result = ok`, 0 errors), and the Inbox / All Mail / Unread views
  render the fetched mail.

## v1.0.1 — planned milestone (not released)

Build-verification work slated for the v1.0.1 milestone, on macOS 26 / Apple Silicon. The first-ever compile of
the OFF-by-default `local-embed` path surfaced three API-drift breakages — the path
is not built in CI, so it had bit-rotted against the resolved `ort`. All three are
fixed; `--features live-net,local-embed` now builds green, as does the default and
`--features live-net` build, the frontend (`tsc && vite build`), and the optimized
release binary.

### Fixed

- **fix(embed):** pin `ort` to `=2.0.0-rc.10` — the `"2.0.0-rc.10"` caret silently
  resolved up to `rc.12`, whose API differs (prerelease-drift guard, mirroring the
  Tauri pin rationale).
- **fix(embed):** `embedding/onnx.rs` against the rc.10 API — drop the `.map_err()?`
  after `ort::inputs!` (the macro returns a `Vec`, not a `Result`); rename
  `try_extract_raw_tensor` → `try_extract_tensor`; hold the ORT `Session` in a
  `Mutex` so inference can borrow it mutably (`Session::run` takes `&mut self`, but
  it was stored in an `Arc`).

### Changed

- **fix(bundle):** move the specta bindings generator from a `[[bin]]` (`gen-bindings`)
  to a `[[example]]` (`gen_bindings`, now at `src-tauri/examples/`). Tauri's bundler
  enumerates every `[[bin]]` and tried to copy the gated helper into the `.app`
  (it isn't built without `specta-export`), failing the bundle; the bundler ignores
  examples. `pnpm gen:types` now runs `cargo run --features specta-export --example
gen_bindings` — bindings output is byte-identical (drift check passes). Refs updated
  in `package.json`, `Cargo.toml`, `CONTRIBUTING.md`. With this,
  `tauri build --bundles app` produces `SeekerMail.app` (ad-hoc/unsigned, runs locally).
  `package.default-run` + `tauri.conf.json > mainBinaryName = "seekermail"` are also set
  for an unambiguous single-bin crate.

### Added

- **ci:** `feature-build.yml` — a weekly (and on-demand) workflow that compiles every
  OFF-by-default feature combo (`live-net`, `+local-embed`, `+local-llm`) on macOS, so
  the kind of silent rot fixed above is caught in CI instead of at release time.

### Known gaps (release verification — need owner action)

- **Unsigned bundle** — `tauri build --bundles app` now produces `SeekerMail.app`
  (ad-hoc signed; runs locally via right-click → Open). Distribution needs a Developer ID
  signature + notarization — no signing identity on the build host, so this needs the
  Apple Developer credentials. The updater-tarball signing key (`TAURI_SIGNING_PRIVATE_KEY`,
  separate from Apple signing) is also unset, so `--bundles app` exits non-zero at the
  updater step _after_ the `.app` is already written.
- **Model assets absent** — real `local-embed` needs the bge-m3 ONNX model; `local-llm`
  needs a `.gguf` model **and** `cmake`. Both run as deterministic offline fakes until
  the assets are placed.
- **`cargo test`** blocks on macOS Keychain GUI prompts (credential tests use the real
  Keychain; the non-macOS path is stubbed). Needs a headless Keychain shim or a
  pre-authorized CI keychain to run unattended. Frontend `vitest`: 169/169 pass.

## v1.0.0 — planned GA scope (not released)

Planned **first commercial GA** milestone (v1.0 GA) — **not yet
shipped**. Source and preview builds are already public; v1.0 is the commercial launch. This section covers the intended v1.0 GA batch (T108–T120). The detailed
pre-GA engineering notes for v0.5–v0.7 appear under the milestone sections below.

### Added — Attachment full-text search (v0.6, T108–T110)

- **feat(extraction):** attachment text extraction — PDF / Word / Excel / PowerPoint /
  plain-text pipeline, migration 011 (renumbered from the card's 008; 008–010 were
  taken). `spawn_blocking` + `catch_unwind` isolation, 200 KB truncation, MIME routing
  with extension fallback, and a `start_attachment_extraction_backfill` command.
- **feat(search):** attachment FTS5 + per-chunk vector index, migration 012
  (`attachments_fts` + triggers + `embedding_att_status`), `build_attachment_index`
  command, and `search_attachments_fts` internal API.
- **feat(ui):** attachment-origin hits in the search panel (distinct card variant) +
  L2 attachment highlight via `?attachmentId=`.

### Added — Cross-account unified search (v1.0 GA, T111–T113)

- **feat(search):** cross-account keyword search (`accountId = null`), M10
  deterministic ranking.
- **feat(search):** cross-account semantic search (`accountId = null` + `accountFilter`),
  M10 deterministic ANN ranking.
- **feat(ui):** unified cross-account search UI — account filter chips, merged
  results, per-account colour badge.

### Added — Windows public beta (v1.0 GA, T114–T116)

- **feat(keychain):** Windows Credential Manager backend — interface parity with the
  macOS Keychain.
- **chore(infra):** Windows packaging — NSIS installer, Authenticode signing helper,
  WebView2 download-bootstrapper, `release.yml` Windows leg + `latest.json`
  `windows-x86_64` entry.
- **chore(infra):** Windows CI matrix leg, cross-platform path/notification/font
  abstractions, `.gitattributes` line-ending normalization, cfg-audit + M11 harness.

### Added — Transaction view / deal tags (v1.0 GA, T119–T120)

- **feat(deal):** G5 deal tags + read-only cross-account aggregation (migration 013;
  no agent memory, P0 isolation pinned by test).
- **feat(deal):** G5 transaction view UI — read-only cross-account timeline with
  source-account colour markers, in-deal search, and jump-back to G3.

### Verification (v1.0 GA, T117–T118)

- **chore(verify):** cross-platform parity audit + GA security audit — parity report,
  security audit, no-proxy Windows validation scripts.
- **chore(release):** v1.0.0-ga release gate — `smoke_v10_ga.sh` + `release_check_v10.sh`,
  GA release notes; first public + commercial milestone.

## v0.5–v0.7 — development milestones (not released)

### v0.5 Beta — Agent-IM / TEAM channel (I module)

The Digital-Employee collaboration surface: a single shared TEAM channel, agent
identity & presence, the master-account invariant, and the event plumbing that
keeps it live. Migrations were renumbered (the cards predate 004–007), the v0.6
`query:*` listeners are wired now so they light up the moment T095/T097 emit, and
the sidebar pending-query badge runs on a focused `count_pending_queries` (the
full query lifecycle is v0.6).

- **feat(accounts):** single-primary enforcement + `set_primary_account` command +
  startup heal (T091). `AccountRepo::set_primary` swaps the flag in one transaction
  (clear all → set one) so "at most one primary" can never break mid-write;
  `create` now promotes a new account only when no primary exists (self-healing a
  primary-less DB rather than only the first row); `heal_primary` runs at startup
  and after every delete, promoting the earliest active account when the count is
  0 or ≥2. Frontend: ★ marker + Primary badge + a focus-trapped "Set as Primary"
  confirmation on `/agents`.
- **feat(im):** `im_messages` schema (`008_im_messages` migration, renumbered from
  the card's 004) + `post_im_message` / `list_im_messages` / `mark_im_message_read`
  commands (T092). `channel_id` is CHECK-pinned to `'main'` (the no-private-chats
  invariant at the data layer), retention prunes >90 days then beyond a 5000-row
  cap fire-and-forget after each insert, and `ImMessage` is specta-exported.
- **feat(agent):** deterministic avatar, presence status, member-change system
  messages (T094). `AgentAvatar` renders a local email-hash identicon (no Gravatar,
  no Canvas — CSP-safe, color from the account token); `AgentNameChip` adds the ★
  and a domain label with a full-email tooltip; `get_agent_statuses` derives
  processing / idle / offline from `sync_state` + recent `ai_drafts`; create/delete
  post a "joined/left the collaboration channel" system message (best-effort).
- **feat(team):** TEAM channel UI — message stream, member drawer, @ mention input
  (T093). `/team` renders the shared channel: system/human/agent bubble layouts
  with logical-property alignment, per-day dividers, auto-scroll, an all-offline
  banner, an empty state, a member drawer with presence dots, an `@`-mention picker
  (keyboard + click), and a failed-send retry strip.
- **feat(events):** `query:new` / `query:expired` + extended `risk:alert`
  listeners, sidebar TEAM badge, content-free push notifications (T101). New
  listeners invalidate the pending-query / channel caches; `notifications.ts` gates
  on the global level (off / priority / all) and degrades to a silent no-op without
  the OS plugin; the sidebar TEAM item shows a red `count_pending_queries` badge.
- **feat(dashboard):** `AgentBadgeRow` with presence chips — processing / idle /
  offline (T102). One compact chip per account on the Dashboard, primary agent
  first, presence dot in design tokens (processing spins), click → `/team`.
- **build(types):** dropped the duplicate `AutoSentPayload` / `AutoLoopDetectedPayload`
  / `PipelineErrorPayload` definitions from the specta provisional appendix — they
  are now registered from `crate::types`, so the hand-written copies produced
  duplicate identifiers on regeneration (found while exporting `ImMessage`/`AgentStatus`).

### v0.6 Beta — Agent-IM proactive queries (I3 / I4)

The proactive-query chain on top of the v0.5 channel: detect T1–T6 situations,
raise a structured QA card, suspend the AI chain until the user answers, and
expire/remind safely. Migrations renumbered (009/010 — the cards predate 005–008).
T3/T6 (AI-assisted) are gated stubs in v0.6; detection stays deterministic and
provider-free. T4 detection bridges off the existing E4 `risk_events`.

- **feat(i3):** proactive query generation T1–T6 + `suspended_i3` mail status
  (T095). Migration `009_mail_processing_status` adds `mails.ai_processing_status`.
  `ai::query_detection` holds the pure rules (T1 unknown-sender-with-risk-keyword,
  T2 meeting-without-a-time, T4 from an open level-4 `risk_events`, T5
  missing-attachment / missing-reply-context); `ai::pipeline::i3_stage::run_i3_detection`
  loads context, writes `pending_queries` + a `query_card` channel message, marks
  the mail `suspended_i3`, and emits `query:new` (+ `risk:alert` for T4). Wired as
  a pre-step in the pipeline worker (before the E1/E2/E3 dispatch); anti-over-notify
  guards: one card per mail, 48 h same-sender dedup, 10/day cap (T4 exempt).
- **feat(i4):** QA card content schema + `generate_qa_card_content` T1–T6 (T098).
  `ai::qa_card` defines `QaCardContent` / `QaCardOption` / `QaCardSubQuestion` /
  `QaCardResponse` (specta-exported), generates a spec-compliant option set per
  trigger (T4 always carries "view original email"; every list ends in Skip), and
  `validate_qa_card_content` enforces 2–4 options + an ≤80 question. The full card
  JSON is stored on both the channel message and `pending_queries.options` so the
  Pending card is self-contained.
- **feat(i3):** `answer_query` / `skip_query` — suspend/resume with conservative
  fallback (T096). `answer_query` transitions the query → `answered`, updates the
  channel card, flips the mail to `analyzing`, and re-queues it (reusing the
  pipeline queue) so the E-chain resumes; `skip_query` writes a per-trigger
  conservative draft to `ai_drafts` — except T4, which keeps the mail suspended
  (never silently dropped). Both run as atomic multi-table transactions.
- **feat(i3):** query expiry 72 h auto-timeout + T4 daily merged reminder (T097).
  Migration `010_query_reminder` adds `pending_queries.last_reminder_at`. A 15-min
  background sweep auto-expires overdue non-T4 queries (same conservative fallback
  as skip; emits `query:expired`) and posts one merged "N unresolved risk alerts"
  reminder per account per day for open T4s (the F5 pressure-relief valve).
- **feat(pending):** DecisionCard QA UI — all states, submit/skip (T099). The
  Pending page's `data-type="decision"` card: trigger badge, open-original-email
  link, question, quick-option chips (single/multi), free-text note, and Submit /
  Skip (with a confirm dialog); state drives the logical left-border (pending →
  interactive, T4 → red, error → amber). Coexists with E2 review drafts via the
  filter chips. New `usePendingQueries` / `useAnswerQuery` / `useSkipQuery` hooks.
- **feat(risk):** T4 non-dismissable risk banner (T100). A sticky red app-shell
  banner (`role="alert"`, no close button — root CLAUDE.md hard rule) appears
  whenever an open level-4 risk event exists, with Review-Now / Open-Email actions
  and a "+N more" count; it clears only when the event is resolved. `risk:alert`
  invalidation makes it appear live.

### v0.5–v0.7 — Compliance, AI safety, and release gates (capstone infra)

Verification + gate infrastructure for the AI batch. The version → tag cut
(`[Unreleased]` → `[0.5.0-beta]` / `[0.6.0-beta]` / `[0.7.0-rc]`) and the
`tasks/INDEX.md` status updates are the PM's step at tag time (the gate cards
explicitly do **not** push tags); these entries are the codeable deliverables.

- **test(compliance):** BYO-AI no-proxy egress + log-safety assertions (T103).
  `src-tauri/tests/compliance/` — `noproxy_egress` asserts AI inference egress
  hosts are the user/provider, never a SeekerMail domain (ADR-0004; host-invariant
  over the built-in defaults + a custom base URL); `log_safety` captures `tracing`
  output with a minimal in-process subscriber and asserts a secret denylist
  (API key / mailbox password / mail body / OAuth token / prompt) never appears,
  plus the key-bearing param `Debug` impls redact to `***` (dev/09 §5). New
  `.github/workflows/compliance.yml` (offline, every push) and
  `docs/compliance/noproxy_check_sop.md` (mitmproxy capture SOP).
- **feat(infra):** AI safety test harness — misfire + sensitive-downgrade gate
  (T104). New `cargo xtask safety-seed | safety-run | safety-gate` over a
  100-mail labelled fixture set (`xtask/src/safety/fixtures/initial_set.json`,
  50 sent_auto / 23 demoted / 27 skipped). `safety-run` emits the stable
  `safety-report.json` (misfire_rate, sensitive_downgrade_rate, skip_accuracy,
  failures); `safety-gate` exits non-zero unless misfire < 5% and downgrade ∈
  [10, 30]% (AI_MODES_DESIGN §11). xtask stays app-crate-free (its supply-chain
  isolation), so the runner mirrors the E4 §5 forced-demotion rules over fixture
  metadata. New `.github/workflows/safety.yml` (weekly + manual). Unit tests
  cover the gate exit codes, the runner metrics, and fixture determinism.
- **chore(release):** v0.5 / v0.6 / v0.7-RC release-gate scripts (T105/T106/T107).
  `scripts/smoke_v05.sh`, `smoke_v06.sh`, `smoke_v07_rc.sh` (automated gates run
  unattended; live-account / app-run E2E cases are `confirm()`-prompted under
  `SMOKE_E2E=1`, mirroring `smoke_v04.sh`), plus `release_check_v07.sh` (RC
  pre-tag checklist: safety gate green, compliance green, smoke clean, unit tests,
  clean tree, CHANGELOG section, evidence files). `docs/releases/` gains the
  evidence README and the E5 blind-test record template.

### v0.6 Beta — draft review (E6)

- **feat(ai):** E2 semi-auto generation backend — needs-reply classifier, pipeline, concurrency (T082)
- **feat(ai):** E2 notification + Pending wiring — throttled OS notify, sidebar badge, L0 draft badge (T083)
- **feat(ai):** draft edit tracking, diff view, and approve-draft → SMTP send integration (T090)
- **feat(ai):** E6 draft queue backend — lifecycle, expiry, IPC commands (T080)
- **feat(ui):** E6 draft review inline in Pending — DraftCard, DraftPanel, filter chips, keyboard shortcuts (T081)

### v0.7 RC — audit log (E7)

- **feat(ui):** E7 audit log UI — AI Activity tab in Report page (T089)
- **feat(ai):** E7 audit log backend — AuditLogger, summary, export, retention (T088)

### v0.7 RC — full-auto safeguards (E3)

- **feat(ai):** E3 full-auto backend — six-point check, 30s delay queue, undo send, rate limits (T085)
- **feat(ai):** E4 sensitive pre-scan — rule chain, LLM fallback, Trash/Sensitive routing (T084)
- **feat(ui):** E3 safeguards — auth level selector, kill switch, auto-send toast/undo, trust downgrade (T086)

### v0.5 Beta — AI auto-reply (E module)

- **feat(ui):** E1 manual AI reply — action bar button, loading states, regenerate in compose (T078)
- **feat(ai):** authorization level enforcement — auth router, guard, IPC settings commands (T087)
- **feat(ai):** draft prompt assembly — DraftPromptBuilder with GTE context + role injection (T079)
- **feat(ai):** E1 manual reply generation backend — draft engine, cleaner, IPC (T077)

### v0.7 RC — style injection (E5)

- **feat(ai):** E5 style injection — style block in prompt builder, cold-start fallback (T076)

### v0.7 RC — provider matrix UI

- **feat(ui):** F4 provider matrix UI — capability×account grid, cell editor, batch ops,
  fine/simplified modes. Settings → AI Providers → Assignment Matrix
  (`/settings/ai/matrix`): a CSS-grid table of the four routable capabilities
  (Draft Reply / Risk Check / Summarize / Style Profile) × every active account, with
  account color-token column headers that collapse to badge-only from four accounts up.
  Each cell shows the primary provider with a local/cloud kind dot and a backup-count
  badge and opens an inline popover editor (primary provider + model, backup chain ≤ 2,
  primary≠backup enforced inline; `VALIDATION` errors render inside the popover without
  closing it). Saves go through `update_provider_matrix`; the returned advisory warnings
  (F_F4 §4.5) render as a non-blocking amber notice list plus an amber cell border +
  tooltip and never block the save. The toolbar carries the fine/simplified mode toggle
  (simplified collapses to one shared "All Accounts" column whose saves batch-overwrite
  every account, with an explicit overwrite notice), reset-to-defaults across accounts
  (`reset_provider_matrix_to_defaults`), and the F_F4 §4.3 batch card — copy row to all
  accounts, copy column to all capabilities, one-click switch of all Risk Checks to a
  configured local provider — all landing as a single `batch_update_provider_matrix`
  call. New `aiMatrix` i18n namespace; hand-written DTO mirrors in `src/ipc/aiMatrix.ts`
  with a stateful off-Tauri mock store that reproduces the backend's default matrix,
  validation, and warning heuristics. (T066)

### v0.5 Beta — BYO AI provider core (F module)

- **feat(ai):** `AiProviderClient` trait, `AiRegistry` routing core, neutral types, `MockProvider`.
  The Module F abstraction layer (dev/06 §2): object-safe `AiProviderClient` (chat /
  chat_stream / health / id / context_window), neutral `ChatRequest`/`ChatResponse`/
  `ChatDelta`/`Capability`/`ProviderError` shapes so nothing vendor-shaped leaks above the
  adapters, and the `AiRegistry` router — per-account factories for cloud adapters,
  account-agnostic singletons for local ones, a fingerprinted per-account client cache, and
  the `daily_query_limit` cost guardrail enforced from `ai_decisions` counts before any
  network call. `ProviderError → AppError` is the single content-safe mapping point
  (a `BadResponse` payload is reduced to its length so a buggy adapter can never leak
  completion text into logs or the wire). ADR-0004 holds by construction: the registry
  stores no SeekerMail server address. Test seam: scripted `MockProvider`. (T058)
- **feat(ai):** OpenAI Chat Completions adapter + `verify_ai_provider` / Module H settings
  commands. `OpenAiClient` (custom `base_url` for OpenAI-compatible gateways, org header,
  connect 10 s / total 60 s, Keychain-frame-only key lifecycle, context-window table),
  wire-exact error mapping (401/403 → Auth, 429 + Retry-After → RateLimited, 400
  context_length_exceeded → ContextTooLong, transport → Unreachable — status-only details,
  never body text), plus `get_account_ai_settings` / `update_account_ai_settings` (aiApiKey
  consumed at the boundary into the OS Keychain; never stored, echoed, or logged) and the
  in-band `verify_ai_provider` probe. (T059)
- **feat(ai):** Anthropic Messages adapter — top-level system field, content-block mapping,
  `anthropic-version` pinning, stop-reason and prompt-too-long classification, same
  key-hygiene and timeout discipline as OpenAI; 200k-class context-window table. (T060)
- **feat(ai):** SSE streaming for OpenAI + Anthropic, shared `ai::sse` parser, retry policy.
  Incremental line-buffered SSE parsing (CRLF and split-chunk safe, 64 KiB line guard that
  never echoes buffered bytes), `data: [DONE]` / `message_stop` termination, mid-stream
  parse failures surfaced as content-free errors, drop-to-cancel semantics; wrapper-level
  `chat_with_retry` (exactly one retry, DraftReply × Unreachable only — risk verdicts stay
  atomic) and `health_with_retry` (2 attempts, jittered). (T061)
- **feat(ai):** Ollama local adapter — OpenAI-compatible localhost route with NDJSON/SSE
  streaming, `/api/tags` model discovery + default-endpoint scan, single-permit inference
  semaphore held across stream lifetime, 120 s local budget, proxy-bypass (`no_proxy`) per
  ADR-0004, no key and no disclosure (local provider). (T062)
- **feat(ai):** in-process local generative adapter (`local_onnx`) — lazy single-load
  lifecycle behind a `GenerativeBackend` seam, deterministic offline backend in the default
  build (mirroring the `local-embed` precedent), real GGUF runtime gated behind the new
  off-by-default `local-llm` feature, model-file discovery in `models/` (manual placement,
  dev/06 §1), idle unload, word-chunked stream. (T063)
- **feat(ai):** F4 provider matrix backend — capability×account routing, backup chain,
  matrix persistence. Migration 006 adds `account_ai_settings.provider_matrix` (JSON
  `CapabilityMatrix`); `AiRegistry::resolve()` consults the capability cell first and falls
  back to the base `ai_provider` columns byte-for-byte unchanged; new `resolve_backup()`
  walks the ≤2-backup chain for F5; commands `get_provider_matrix` (NULL → computed
  defaults, local_onnx preferred for RiskReason/StyleProfile), `update_provider_matrix`
  (validation + non-blocking F_F4 §4.5 advisory warnings), `reset_provider_matrix_to_defaults`,
  `batch_update_provider_matrix`. (T065)
- **feat(ai):** F5 `FallbackRouter` — failure classification with in-place retries,
  per-provider cooldowns honoring `Retry-After`, deterministic backup-chain traversal,
  E3→E2 downgrade decisions (never skip, never send blind), in-memory hold queue with
  bounded throttled catch-up, global-AI-offline short-circuit with `ai:offline`/`ai:online`
  events and lead-window recovery probes, `set_ai_disabled` user kill switch, and an
  append-only `ai_decisions` audit row per invocation. (T067)
- **fix(db):** migration 007 replaces the `idx_decisions_today` partial index — its
  `strftime('%s','now')` WHERE clause is rejected by SQLite as non-deterministic on every
  `ai_decisions` INSERT, which made the audit table append-proof; replaced with a plain
  `created_at` index. (found while landing T058/T067)

### v0.5 — AI roles (D module)

- **feat(ai):** role context assembly — the dev/06 §5 grounded-prompt builder.
  `assemble_role_context` produces role + safety preambles from `accounts.role_type`/
  `role_description`, GTE top-K context via vector ANN (two-stage recall, per-mail score
  aggregation, `knowledge_refs` recorded for audit), recent-thread snippets, and contact
  history, packed to the model budget in fixed priority order (safety > target mail >
  thread > GTE context) with `AI_CONTEXT_TOO_LONG` only when the irreducible minimum
  doesn't fit. (T074)
- **feat(ai):** D1 legal role backend — `analyze_legal_risk` command, `LegalAnalysisPipeline`
  (legal persona system prompt, temperature 0.0, strict D1 JSON schema validation with one
  re-prompt retry, oversize-mail segmentation + merge), transactional `risk_events` +
  `ai_decisions` writes (evidence stores a hash prefix, never the excerpt), 24 h result
  cache that spends no quota on hits. (T070)
- **feat(ai):** D2 sales role backend — `analyze_sales_context` command,
  `SalesAnalysisPipeline` (stance-aware consultant prompt, counterparty profile / needs /
  concession advice / next actions with strict schema validation, marketing-sender guard,
  contact-history grounding, 24 h cache, content-free audit rows). (T072)
- **feat(ai):** E5 style learning — sent-mail sampler (180-day window, filtered and capped),
  two-stage LLM profiler into the F_E5 §4.2 `style_profile` JSON (+ samples count, pinned
  guard), `trigger_style_learning` command with single-flight dedup, `style:progress`/
  `style:done`/`style:error` events, and a 30-day background refresh worker. (T075)

- **feat(ai):** F3 recommended provider one-click setup wizard + OAuth + disclosure modal +
  conservative quota. New `ai::recommended` config module (two tiers — balanced / high-quality —
  vendors are v0.5 provisional constants per F_F3 §4.1; endpoints, scopes, and client-id env
  names live in `RECOMMENDED_PROVIDERS`, never in flow code) with a PKCE grant mirroring T015:
  system browser → `seekermail://oauth/recommended` deep link (distinct path from the
  account-mail callback so the handler can route without guessing) → CSRF state validation →
  direct device-to-vendor token exchange → token into the OS Keychain per account → F4 default
  matrix fill (E4 keeps `local_onnx` when registered) → in-band connection probe. Commands:
  `get_recommended_providers`, `begin_recommended_oauth`, `complete_recommended_oauth` (named
  to avoid colliding with the T015 account `begin/complete_oauth_flow`),
  `revoke_recommended_provider`, `get_ai_setup_status`, `confirm_ai_disclosure`,
  `clear_conservative_quota`. The dev/06 §8 data-flow disclosure is non-bypassable end-to-end:
  the modal has no dismiss path besides its two explicit buttons, and the backend refuses to
  begin a cloud grant until `ai.disclosure_confirmed_at` exists. First authorization stamps
  `ai.first_auth_at` and arms a 7-day conservative quota (`ai.conservative_quota_until`):
  `AiRegistry::resolve`/`resolve_backup` cap the daily limit at 100 and the new
  `AiRegistry::clamp_chat_request` caps `max_tokens` at 2000 while armed; the settings surface
  lifts it early via `clear_conservative_quota`. Frontend: `RecommendedSetupWizard` (three-card
  entry → tier cards with monthly-cost estimates → authorizing with deep-link listener + manual
  code fallback → connection test → Ready, and the F_F3 §5 failure surface with Retry / my-key /
  local-model exits) mounted at `/settings/ai/recommended`; reusable `DataFlowDisclosureModal`;
  `aiSetup` i18n namespace. (T064)
- **feat(ui):** AI provider config UI — cloud add wizard, local Ollama discovery, per-account
  automation level. Settings → AI Providers replaces the T073 stub: a configured-provider list
  (account color token, provider type + model, "🔒 Local" badge for ollama/local_onnx, status
  badge with local retest, Edit / Remove actions) fed by the new `list_configured_providers`
  command; an Add-Cloud wizard (Anthropic / OpenAI / OpenAI-compatible / Azure / Gemini — the
  latter three ride the `openai` wire variant with a custom base URL) with type → credentials →
  in-band `verify_ai_provider` test (401 / 404 / unreachable / rate-limit copy per F_F1 §4.3) →
  model + account fan-out; an Add-Local wizard (new `scan_local_providers` endpoint discovery +
  manual URL, `list_ollama_models` model list with size/quantization metadata, verify, save).
  The API key lives only in form state on its way to `update_account_ai_settings` (Keychain
  write at the boundary), is cleared the moment save starts, and is never echoed — editing
  shows a masked placeholder. EditAccountSheet gains the E1/E2/E3 automation-level control
  with a Full-Auto confirmation intercept that mirrors `auth_level` into
  `account_ai_settings` after the accounts row updates. The "Recommended Setup" entry links
  to `/settings/ai/recommended` (T064's wizard mounts there). (T068)
- **feat(ai):** data-flow disclosure panel — real AI routing (replaces the v0.4 placeholder);
  content-safe `ai_decisions` audit log. New `get_data_flow_ai_routing` command reports, per
  account, the configured provider and its _real_ effective endpoint (`ai_base_url` override or
  the adapter default — `api.openai.com` / `api.anthropic.com` / `localhost:11434` / in-process),
  a cloud/local/in-process/off classification with `is_local` flag, and a 24 h `ai_decisions`
  activity summary (counts + token totals only — never prompt, completion, or mail content).
  The Data Flow panel's amber "No AI requests in v0.4" notice is replaced by `AiRoutingSection`:
  per-account rows (cloud rows carry the "mail content is sent to this endpoint" disclosure,
  local rows show "On this device", AI-off rows render muted), the fixed ADR-0004 statement
  "SeekerMail servers are never in the path", and the 24 h activity card. (T069)
- **feat(ui):** D1 legal risk sidebar — risk list, key clauses, T4 non-dismissable banner,
  report risk panel. Legal tab in the L2 ThreadDrawer (replaces the T041 placeholder) with
  lazy 24 h-cached `analyze_legal_risk`, severity-sorted risk items whose excerpts highlight
  the matching body text post-DOMPurify (`mark.legal-highlight`), collapsible key clauses,
  compliance advice, resident disclaimer, and provider-gap fallback linking to /agents. T4
  open risk events render as stacked `role="alert"` banners above the mail header with no
  close affordance — "Mark Resolved" is the only action; `risk:alert` push invalidation makes
  them appear live. `/report` gains the open risk-events panel (T4 rows offer Resolve only,
  no Dismiss). `list_risk_events`/`resolve_risk_event` run on hand-written DTO mirrors
  (`src/ipc/legal.ts`) + the dev mock layer until the Module E command surface is registered.
  (T071)
- **feat(ui):** /agents role config & persona editor — `role_type`, `role_description`,
  `auth_level` with full-auto confirmation. One card per account (role-type accent line, account
  badge), 500-char soft limit on the description, three-tier auth segmented control where
  switching to Full Auto is intercepted by a focus-trapped confirmation dialog. Save order:
  `update_account` then `update_account_ai_settings` (auth_level mirror). Read-only AI-provider
  status row links to the new `/settings/ai` page (provider config lands with the F-module
  cards). The AI-settings commands run on hand-written DTO mirrors (`src/ipc/aiSettings.ts`) +
  the dev mock layer until T059's command surface is registered. (T073)

## [0.4.0-beta] - 2026-06-12

The v0.2–v0.4 MVP batch (T013–T057): a working local-first mail client —
accounts & sync, storage, sanitisation, GTE search, compose/send, the full
settings & data-management surface, and the perf + release infrastructure.

- **chore(release):** v0.4.0-beta release gate — `scripts/smoke_v04.sh` (automated gates +
  the five §4.2 E2E cases under `SMOKE_E2E=1`), CHANGELOG finalisation, CONTRIBUTING
  recalibrated to v0.4 (bench/release commands, module map, perf-gate DoD). (T057)
- **feat(infra):** performance benchmark harness — M1–M8 gate. `cargo xtask bench-seed`
  (deterministic 100k corpus, seed=42, checksum-asserted), `bench` (per-metric harnesses with
  smoke mode), `bench-gate` (threshold red exit(1) / baseline×1.10 amber), committed
  `bench-baseline.json`, nightly `bench.yml` on the Tier-A runner. (T055)
- **chore(infra):** macOS packaging, signing, notarization & updater pipeline —
  `release.yml` (aarch64 + x86_64 matrix; cert import → build → inside-out codesign with
  `--timestamp` + hardened runtime → DMG → `notarytool --wait` + staple → `spctl` verify →
  `latest.json` + GitHub Release), minimal `entitlements.plist` (network.client only),
  `deny.toml` license/advisory gate, `scripts/release_check.sh` pre-tag checklist. (T056)

### v0.3 / v0.4 — Settings & data management (T050–T054)

- **feat(settings):** appearance — light/dark/system theme toggle. `ui.theme` persists to
  `app_settings` via new `get_setting`/`set_setting` commands; `html.dark` token overrides in
  `tokens.css`; FOUC guard (backend-injected `__INITIAL_THEME__` + `theme-boot.ts`); live
  follow-system via `prefers-color-scheme` listener. (T050)
- **feat(settings):** privacy controls — tracker protection & remote image policy. Three-level
  `TrackerPolicy`/`ImagePolicy` enums, `apply_privacy_policy` command, first-run defaults seeded
  in setup (`block_known` / `block_all` — protection ON), content-free
  `privacy_policy_changed` log event, Reset-to-Defaults confirm dialog. (T051)
- **feat(data):** export to mbox and JSON Lines. `start_export`/`cancel_export` background task
  (batched 500-row reads, disk-space guard ×1.1 → `FS_DISK_FULL`, `export:*` events), RFC 4155
  mbox writer with `>From` escaping, JSONL + `MANIFEST.json`, in-tree STORED-zip packaging,
  four-step wizard UI with event-driven progress + cancel + Open in Finder. Credentials are
  structurally absent from exports. (T052)
- **feat(data):** wipe, reindex, and sync-range controls. Wipe: batched deletes + typed-DELETE
  guard + last-account `FORBIDDEN` rail + VACUUM with freed-bytes report (`wipe:*` events).
  Reindex: checkpoint-resumable rebuild (vectors + FTS5 `rebuild`), A4 polling + embed worker
  paused for the run with always-resume, 5 % sample verification, completion report persisted to
  `app_settings`. Sync range: grow flags `full_sync_required`, shrink previews then deletes
  out-of-range local rows. Data & Storage hub page routes to all sub-pages. (T053)
- **feat(settings):** data-flow disclosure panel (v0.4 — local-only). Fully static six-row
  panel driven by a `DATA_FLOWS` const; amber "No AI requests in v0.4" notice; RTL-mirrored
  inline arrow; token-only colors. (T054)

### v0.3 / v0.4 — Search (GTE), compose/send, and the full reading UI (T030–T049)

Backend — the GTE embedding + search core, plus send & drafts:

- **feat(embedding):** ONNX embed runtime, bge-m3, 1024-dim, SHA-256 guard. The real
  `ort` runtime is behind the off-by-default `local-embed` feature; the default build ships a
  deterministic offline embedder (feature-hashing) so the whole B3 pipeline runs without the
  2.2 GB model — mirroring T019's LanceDB containment. (T030)
- **feat(embedding):** chunking + bounded embed queue + per-chunk vector upsert pipeline,
  `embedding_status` lifecycle, retry/catch-up, `gte:*` progress events. (T031)
- **feat(search):** `keyword_search` FTS5 command, DSL parser (`from:/to:/subject:/in:/has:` +
  booleans + quoted phrases), BM25 + time-decay ranking, `<mark>` highlights. (T032)
- **feat(search):** `semantic_search` ANN command — two-stage retrieval (SQLite pre-filter →
  vector ANN), per-mail aggregation + cosine rerank + 0.35 threshold gate. (T033)
- **feat(search):** saved searches + search history (`save_search`, `delete_saved_search`,
  `list_saved_searches`, `get_search_history`). (T035)
- **feat(send):** SMTP `send_mail` with a 10 s `cancel_send` window (`tokio::select!` + oneshot),
  SENT persistence + thread association, `mail:new` event. Transport behind `live-net`
  (`lettre`); offline build accepts the message so the flow is exercised end-to-end. (T043)
- **feat(drafts):** compose-draft persistence (`save_draft`/`get_draft`/`delete_draft`) on a new
  `compose_drafts` table (migration `005`). (T045)

Frontend — search, reading, compose, and the shell:

- **feat(ipc):** mail-list query hooks (infinite threads/mails, detail, mutations), search +
  draft hooks, `gte:*` event wiring, store extensions (multi-select, L1 filter, thread folding,
  compose buffer). (T036)
- **feat(ui):** global Cmd+K search panel — keyword/semantic toggle, debounced results,
  highlights, history, save-search dialog; saved searches in the sidebar. (T034/T035)
- **feat(ui):** L0 virtualized card stream, bulk actions + undo, keyboard shortcuts, L1
  folder/filter drawer, thread folding. (T037–T040)
- **feat(ui):** L2 reading view + detail attachment list. (T041/T042)
- **feat(ui):** compose editor (recipients, toolbar, body, attachments bar, footer) with send +
  10 s undo and debounced draft autosave. (T044/T045)
- **feat(ui):** onboarding gate, unread/processed/all-mail routes, settings shell + nav
  (accounts/appearance/privacy/data/about). (T046–T049)

> Note: the mail-list **read** backend (`list_threads`/`list_mails`/`get_mail`) is not part of
> this batch; those surfaces run on the dev mock layer until that card lands. Search, send, and
> drafts call the real backend.

### v0.2 — MVP accounts, storage & mail fetch + v0.4 B1/B2 (T013–T029)

The first product-behavior slice: real accounts (CRUD, OAuth, connection probe),
the three-layer storage facade, the IMAP fetch pipeline (scheduler → backfill →
parse → persist), attachments, HTML sanitisation, and the accounts/reading-view UI.
Network transports (IMAP/SMTP/OAuth-HTTP) sit behind a test seam — the default
build wires offline fakes; `--features live-net` enables the concrete adapters.

- **feat(accounts):** account CRUD backend — `AccountRepo`, `AccountService`,
  8 IPC commands; create writes `accounts`+`sync_state`+`account_ai_settings` in one
  transaction; passwords go to the Keychain, never the DB. (T013)
- **feat(accounts):** IMAP/SMTP connection probe (in-band result, 15 s timeout) +
  provider autoconfig presets. (T014)
- **feat(accounts):** Google + Microsoft OAuth 2.0 PKCE grant + Keychain storage,
  zeroized tokens, CSRF `state` + 5-minute pending TTL. (T015)
- **feat(accounts):** mailbox sampling + knowledge-depth selection backend
  (`002_knowledge_depth` migration). (T016)
- **feat(ui):** accounts settings — list, four-state badges, add wizard with the
  knowledge-depth step, edit sheet, IPC hooks. (T017)
- **feat(accounts):** OAuth token refresh, `auth_failed` detection, `reauth_account`
  command; concurrent refresh serialised per account. (T018)
- **feat(storage):** vector index + three-layer `StorageFacade` (SQLite authoritative,
  derived vectors, disk blobs). _Note:_ v0.2 ships a brute-force cosine backend behind
  the `VectorStore` API in place of LanceDB (see report). (T019)
- **feat(storage):** `DiskBlobStore` — attachment file layout, disk accounting,
  exec-block guard, 500 MB low-watermark. (`004_attachment_available` migration). (T020)
- **feat(imap):** polling scheduler with per-account tasks, concurrency cap 4,
  exponential backoff. (T021)
- **feat(imap):** history backfill (knowledge-depth, resumable, throttled) +
  incremental UIDNEXT fetch (`003_backfill_state` migration). (T022)
- **feat(imap):** MIME parse worker + thread resolution + `upsert_batch` +
  `mail:new` events. (T023)
- **feat(events):** typed Tauri event emitter + frontend TanStack Query invalidation
  wiring. (T024)
- **feat(attachments):** streaming IMAP download, SHA-256 dedup (hard-link),
  50 MB cap, concurrency limits. (T025)
- **feat(attachments):** open/reveal OS integration, orphan cleanup on
  account/mail delete. (T026)
- **feat(sanitize):** ammonia ingest pipeline — B1/B2 first pass, tracker count,
  `body_text`. (T027)
- **feat(ui):** `SanitizedMail` component — DOMPurify second pass, token styling. (T028)
- **feat(mail):** tracker badge + remote-image block bar (B2). (T029)

### v0.1 — Internal engineering skeleton (T001–T012)

This release stands up the bootable scaffold: a Tauri 2 + Rust + React 18 monorepo
with the cross-cutting foundations (types, errors, logging, storage, credentials,
shell, i18n, CI). No product behavior (mail fetch, search, AI) ships yet — those
begin at v0.2.

- **chore(scaffold):** monorepo & Tauri app shell — pnpm workspace, pinned
  toolchains (`rust-toolchain.toml`, `.nvmrc`, `pnpm@9`), bootable window titled
  "SeekerMail", frozen directory layout. (T001)
- **feat(ipc):** first `invoke` roundtrip + command-registration pattern — `ping`
  command, one-module-per-file `commands/`, the single `src/ipc/` data layer. (T002)
- **build(types):** specta Rust→TS bindings pipeline + drift check —
  `packages/shared/src/bindings.ts` generated from Rust DTOs via `pnpm gen:types`. (T003)
- **feat(error):** `AppError`/`IpcError` model + structured logging — exhaustive
  `code()`, single boundary conversion + log point, `tracing` line-JSON with a
  secret denylist, frontend `errors.ts` mapping. (T004)
- **feat(storage):** sqlx pool, per-connection PRAGMAs, forward-only migrations +
  `001_init` (14 tables + `mails_fts`). (T005)
- **feat(keychain):** macOS credential vault (set/get/delete) keyed
  `{account_id}:{kind}`; zeroized, redacted secrets. (T006)
- **feat(ui):** app shell, routing, state, token binding — three-region shell,
  14 routes, Zustand + TanStack Query, Tailwind→Seeker tokens. (T007)
- **feat(i18n):** react-i18next scaffolding, English default, 21-locale metadata +
  RTL/script font stacks. (T008)
- **ci:** GitHub Actions (macOS-14) — lint/type/test + bindings drift gate. (T009)
- **build(model):** idempotent ONNX model fetch script (bge-m3, 1024-dim) with
  trust-on-first-use `model.lock.json`. (T010)
- **chore(config):** `.env.example`, `.gitignore`, pre-commit hooks
  (prettier + eslint + rustfmt) and Conventional-Commits commitlint. (T011)
- **docs:** CONTRIBUTING mirror of `dev/08`; **chore:** v0.1 skeleton smoke gate. (T012)

[Unreleased]: https://github.com/seekermail/seekermail-desktop/commits/main
