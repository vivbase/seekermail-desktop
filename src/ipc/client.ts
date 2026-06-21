// THE single data-access layer (07 §6). This is the ONLY module in `src/` allowed
// to import `@tauri-apps/api`; everything else imports the hooks in `queries/`.
// ESLint enforces this boundary (eslint.config.js).
import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  AiDecisionRow,
  AiDraft,
  ApproveDraftResult,
  Attachment,
  AttachmentHit,
  AttachmentIndexBuildStatus,
  BackfillStatus,
  CancelSendResult,
  CreateAccountParams,
  DecisionSummary,
  DiskUsage,
  Draft,
  ExportAiDecisionsParams,
  ExtractionBatchStarted,
  ListAiDraftsParams,
  ListDecisionsParams,
  ImageAllowScope,
  ImagePolicy,
  IpcError,
  KeywordSearchParams,
  ListMailsParams,
  ListThreadsParams,
  MailDetail,
  MailSummary,
  OAuthBeginResult,
  SeekerMailId,
  PageResult,
  PingReply,
  Provider,
  RegenerateDraftParams,
  RequestAiReplyParams,
  SamplingResult,
  SaveDraftParams,
  SavedSearch,
  SaveSearchParams,
  SearchHistoryItem,
  SearchResult,
  SearchWithAttachmentsParams,
  SearchWithAttachmentsResult,
  SemanticSearchParams,
  SendMailParams,
  SendMailResult,
  StartExportParams,
  SyncRangePreview,
  SyncState,
  Thread,
  TrackerInfo,
  TrackerPolicy,
  UpdateAccountParams,
  VerifyConnectionParams,
  VerifyConnectionResult,
  WipePreview,
  WipeScope,
} from "@shared/bindings";
// Hand-written DTO mirrors for the AI-settings commands (T073) and the T068
// provider-config surface; replaced by the generated bindings once the Rust
// command surface is exported via `pnpm gen:types`.
import type {
  AccountAiSettings,
  AiProvider,
  ConfiguredProviderInfo,
  ListCloudModelsParams,
  LocalProviderEndpoint,
  OllamaModelEntry,
  UpdateAiSettingsParams,
  VerifyAiProviderParams,
  VerifyAiProviderResult,
} from "./aiSettings";
// Hand-written DTO mirrors for the F4 provider matrix (T065/T066); replaced by
// the generated bindings once the matrix command surface is exported.
import type {
  BatchMatrixUpdate,
  Capability,
  CapabilityMatrix,
  MatrixCell,
  MatrixEntry,
  MatrixWarning,
  ProviderAssignment,
} from "./aiMatrix";
// Hand-written DTO mirrors for the T069 data-flow AI routing disclosure;
// replaced by the generated bindings once the command surface is exported.
import type { DataFlowAiRouting } from "./dataFlow";
// Hand-written DTO mirrors for the GTE stats + topic-breakdown commands (commands/gte.rs).
import type { GteStats, KnowledgeEntry, TopicCount } from "./gteStats";
// Hand-written DTO mirror for the Agent-IM channel (T092); replaced by the
// generated bindings once the im command surface is exported via gen:types.
import type { ImMessage } from "./im";
// Hand-written DTO mirror for Agent presence (T094).
import type { AgentStatus } from "./agents";
// Hand-written DTO mirror for proactive queries (T095/T096/T099).
import type { PendingQuery } from "./pendingQueries";
// Hand-written DTO mirrors for the D1 legal analysis + risk events (T071);
// replaced by the generated bindings once T070's surface is exported.
import type {
  AnalyzeLegalRiskParams,
  LegalAnalysisResult,
  ListRiskEventsParams,
  ResolveRiskParams,
  RiskEvent,
} from "./legal";
// Hand-written DTO mirrors for the F3 recommended-provider setup (T064);
// replaced by the generated bindings once the T064 surface is exported.
import type {
  AiSetupStatus,
  BeginRecommendedOAuthResult,
  CompleteRecommendedOAuthResult,
  RecommendedProviderInfo,
} from "./recommended";

/**
 * The typed command surface. Each entry maps a command name to its `input`/
 * `output` types from the generated `@shared/bindings` (T003). Argument KEYS are
 * the Rust parameter identifiers (snake_case); the values (params/patch) are the
 * camelCase DTOs. The map mirrors the Rust `generate_handler!` list.
 */
export type Commands = {
  ping: { input: undefined; output: PingReply };

  // Accounts (T013)
  list_accounts: { input: undefined; output: Account[] };
  get_account: { input: { account_id: string }; output: Account };
  create_account: { input: { params: CreateAccountParams }; output: Account };
  update_account: {
    input: { account_id: string; patch: UpdateAccountParams };
    output: Account;
  };
  delete_account: { input: { account_id: string }; output: null };
  // SeekerMail ID identity (A6, decoupled from mailboxes). Sign-out clears only the
  // identity; mailboxes and local mail are untouched. Google sign-in is stubbed in
  // the backend until the cloud-identity service ships (T121).
  sign_out_seekermail: { input: undefined; output: null };
  get_seekermail_id: { input: undefined; output: SeekerMailId | null };
  set_marketing_consent: {
    input: { consent: boolean; source: string | null };
    output: SeekerMailId | null;
  };
  begin_google_signin: { input: undefined; output: OAuthBeginResult };
  complete_google_signin: {
    input: { code: string; state_nonce: string };
    output: SeekerMailId;
  };
  update_account_password: {
    input: { account_id: string; password: string };
    output: null;
  };
  enable_account: { input: { account_id: string }; output: Account };
  disable_account: { input: { account_id: string }; output: Account };
  // Master account (T091): promote one account to primary. The single-primary
  // invariant is enforced atomically in the backend transaction.
  set_primary_account: { input: { account_id: string }; output: Account };

  // Account AI settings (T073, dev/02 §Module H). Backed by the real Rust
  // commands (commands::ai::{get,update}_account_ai_settings, registered in
  // lib.rs); the mock layer below is only the off-Tauri dev/test double. The
  // hook surface (queries/accounts.ts) is final.
  get_account_ai_settings: {
    input: { account_id: string };
    output: AccountAiSettings;
  };
  update_account_ai_settings: {
    input: { account_id: string; params: UpdateAiSettingsParams };
    output: AccountAiSettings;
  };

  // BYO-AI provider config (T068, dev/02 §Module H) — real backend commands.
  verify_ai_provider: {
    input: { params: VerifyAiProviderParams };
    output: VerifyAiProviderResult;
  };
  scan_local_providers: { input: undefined; output: LocalProviderEndpoint[] };
  list_ollama_models: {
    input: { base_url: string | null };
    output: OllamaModelEntry[];
  };
  list_cloud_models: {
    input: { params: ListCloudModelsParams };
    output: string[];
  };
  list_configured_providers: {
    input: undefined;
    output: ConfiguredProviderInfo[];
  };

  // F4 provider matrix (T065/T066, F_F4) — real backend commands.
  get_provider_matrix: {
    input: { account_id: string };
    output: CapabilityMatrix;
  };
  update_provider_matrix: {
    input: { account_id: string; matrix: CapabilityMatrix };
    output: MatrixWarning[];
  };
  reset_provider_matrix_to_defaults: {
    input: { account_id: string };
    output: CapabilityMatrix;
  };
  batch_update_provider_matrix: {
    input: { updates: BatchMatrixUpdate[] };
    output: null;
  };

  // Data-flow disclosure (T069, dev/06 §8) — real backend command.
  get_data_flow_ai_routing: { input: undefined; output: DataFlowAiRouting };

  // Agent-IM / TEAM channel (T092) — real backend commands. Argument keys are
  // the Rust parameter identifiers (snake_case); `channel_id` must be "main".
  post_im_message: {
    input: {
      channel_id: string;
      sender_type: string;
      sender_id: string;
      message_type: string;
      content: string;
      linked_email_id: string | null;
    };
    output: ImMessage;
  };
  list_im_messages: {
    input: {
      sender_id: string | null;
      status: string | null;
      limit: number | null;
      offset: number | null;
    };
    output: PageResult<ImMessage>;
  };
  mark_im_message_read: { input: { id: string }; output: null };
  // Mark the entire shared channel read — what opening the TEAM page does.
  mark_im_channel_read: { input: { channel_id: string }; output: null };
  // Count of pending I3/I4 queries (still drives the Dashboard pending tile).
  count_pending_queries: { input: undefined; output: number };
  // TEAM nav badge — unread agent messages + unresolved decision cards (T101).
  count_team_unread: { input: undefined; output: number };

  // Agent presence (T094) — derived statuses for every active account.
  get_agent_statuses: { input: undefined; output: AgentStatus[] };

  // Proactive queries (T096/T099) — list + answer/skip the I3/I4 cards.
  list_pending_queries: {
    input: { account_id: string | null };
    output: PendingQuery[];
  };
  answer_query: { input: { id: string; answer: string }; output: null };
  skip_query: { input: { id: string }; output: null };

  // F3 recommended-provider one-click setup (T064) — real backend commands.
  // Distinct from the account-mail `begin/complete_oauth_flow` (T015).
  get_recommended_providers: {
    input: undefined;
    output: RecommendedProviderInfo[];
  };
  get_ai_setup_status: { input: undefined; output: AiSetupStatus };
  confirm_ai_disclosure: { input: undefined; output: AiSetupStatus };
  clear_conservative_quota: { input: undefined; output: null };
  begin_recommended_oauth: {
    input: { tier: string };
    output: BeginRecommendedOAuthResult;
  };
  complete_recommended_oauth: {
    input: { state_nonce: string; code: string };
    output: CompleteRecommendedOAuthResult;
  };
  revoke_recommended_provider: { input: { tier: string }; output: null };

  // Connection probe (T014)
  verify_account_connection: {
    input: { params: VerifyConnectionParams };
    output: VerifyConnectionResult;
  };

  // OAuth (T015/T018)
  begin_oauth_flow: {
    input: { provider: Provider; account_id: string };
    output: OAuthBeginResult;
  };
  complete_oauth_flow: {
    input: { code: string; state_nonce: string };
    output: null;
  };
  reauth_account: {
    input: { account_id: string; password: string | null };
    output: null;
  };

  // Knowledge depth + sampling (T016)
  sample_mailbox: { input: { account_id: string }; output: SamplingResult };
  set_knowledge_depth: {
    input: { account_id: string; months: number | null };
    output: Account;
  };

  // Disk usage (T020)
  get_account_disk_usage: { input: { account_id: string }; output: DiskUsage };

  // Sync (T021)
  trigger_sync: { input: { account_id: string }; output: null };
  get_sync_state: { input: { account_id: string }; output: SyncState };

  // Backfill (T022)
  get_backfill_status: {
    input: { account_id: string };
    output: BackfillStatus;
  };
  pause_backfill: { input: { account_id: string }; output: null };
  resume_backfill: { input: { account_id: string }; output: null };

  // Attachments (T025/T026)
  download_attachment: { input: { attachment_id: string }; output: string };
  get_attachments_for_mail: {
    input: { mail_id: string };
    output: Attachment[];
  };
  open_attachment: { input: { attachment_id: string }; output: null };
  reveal_attachment: { input: { attachment_id: string }; output: null };
  get_attachment_local_path: {
    input: { attachment_id: string };
    output: string | null;
  };

  // Shell / external links. Opens http/https/mailto/tel in the OS default app
  // (browser / mail client) so a link click in rendered mail HTML never
  // navigates the app's own webview away from the SPA. The backend re-validates
  // the scheme (commands/shell.rs).
  open_external_url: { input: { url: string }; output: null };

  // Tracker / remote images (T029)
  get_tracker_info: { input: { mail_id: string }; output: TrackerInfo };
  allow_remote_images: {
    input: { mail_id: string; scope: ImageAllowScope };
    output: null;
  };

  // Search (T032/T033/T035) — real backend
  keyword_search: {
    input: { params: KeywordSearchParams };
    output: PageResult<SearchResult>;
  };
  semantic_search: {
    input: { params: SemanticSearchParams };
    output: PageResult<SearchResult>;
  };
  get_search_history: {
    input: { limit: number | null };
    output: SearchHistoryItem[];
  };
  list_saved_searches: { input: undefined; output: SavedSearch[] };
  save_search: { input: { params: SaveSearchParams }; output: SavedSearch };
  delete_saved_search: { input: { id: string }; output: null };

  // GTE index stats + topic breakdown (Repository / GTE pages) — real backend
  // commands (commands/gte.rs); served by the mock layer below off-Tauri.
  get_gte_stats: { input: undefined; output: GteStats };
  get_topic_breakdown: { input: undefined; output: TopicCount[] };
  list_knowledge_entries: {
    input: { account_id: string | null; limit: number | null };
    output: KnowledgeEntry[];
  };

  // Attachment-hit search (T110) + extraction/index control (T108/T109)
  search_with_attachments: {
    input: { params: SearchWithAttachmentsParams };
    output: SearchWithAttachmentsResult;
  };
  start_attachment_extraction_backfill: {
    input: undefined;
    output: ExtractionBatchStarted;
  };
  build_attachment_index: { input: undefined; output: AttachmentIndexBuildStatus };

  // Compose / send (T043) — real backend
  send_mail: { input: { params: SendMailParams }; output: SendMailResult };
  cancel_send: { input: { pending_id: string }; output: CancelSendResult };

  // Drafts (T045) — real backend
  save_draft: { input: { params: SaveDraftParams }; output: Draft };
  get_draft: { input: { id: string }; output: Draft };
  delete_draft: { input: { id: string }; output: null };

  // AI drafts (Module E: T077 E1 generation, T080 E6 queue). Backed by the real
  // Rust commands (commands::ai::{request_ai_reply, regenerate_draft,
  // list_pending_drafts, get_ai_draft, update_draft_body, approve_draft,
  // discard_draft, cancel_draft_send}, registered in lib.rs); the mock layer
  // below is only the off-Tauri dev/test double. The AI-draft getter is
  // `get_ai_draft` (NOT dev/02's `get_draft`) to avoid colliding with the T045
  // compose-draft command of the same name.
  request_ai_reply: { input: { params: RequestAiReplyParams }; output: AiDraft };
  regenerate_draft: { input: { params: RegenerateDraftParams }; output: AiDraft };
  list_pending_drafts: { input: { params: ListAiDraftsParams }; output: AiDraft[] };
  get_ai_draft: { input: { id: string }; output: AiDraft };
  update_draft_body: { input: { id: string; body_current: string }; output: AiDraft };
  approve_draft: { input: { id: string }; output: ApproveDraftResult };
  discard_draft: { input: { id: string; reason: string | null }; output: null };
  // T090 backstop: rolls a still-queued draft back to `pending` before the
  // SMTP send actually ran; CONFLICT once the draft is `sent`.
  cancel_draft_send: { input: { id: string }; output: AiDraft };

  // E7 audit log (T088 backend, T089 UI). Backed by the real Rust commands
  // (commands::ai::{list_ai_decisions, get_ai_decisions_summary,
  // export_ai_decisions}, registered in lib.rs); the mock layer below is only
  // the off-Tauri dev/test double. Argument keys follow the T088 contract
  // verbatim (camelCase for the summary command).
  list_ai_decisions: { input: { params: ListDecisionsParams }; output: AiDecisionRow[] };
  get_ai_decisions_summary: {
    input: { accountId: string | null; sinceUnix: number; untilUnix: number };
    output: DecisionSummary;
  };
  export_ai_decisions: { input: { params: ExportAiDecisionsParams }; output: string };

  // Settings (T050/T051)
  get_setting: { input: { key: string }; output: string | null };
  set_setting: { input: { key: string; value: string }; output: null };
  // Global AI master switch (T067, F_F5 §4.5). Disable every AI capability until
  // a unix-seconds deadline, or null to restore immediately. The fallback router
  // honors `ai.disable_until`; reads go through `get_setting`. Hook: queries/settings.ts.
  set_ai_disabled: { input: { until: number | null }; output: null };
  apply_privacy_policy: {
    input: { tracker_policy: TrackerPolicy; remote_image_policy: ImagePolicy };
    output: null;
  };

  // Export (T052)
  start_export: { input: { params: StartExportParams }; output: string };
  cancel_export: { input: { task_id: string }; output: null };
  open_export_output: { input: { task_id: string }; output: null };

  // Wipe / reindex / sync range (T053)
  preview_wipe: { input: { account_ids: string[] }; output: WipePreview };
  start_wipe: {
    input: { account_ids: string[]; scope: WipeScope };
    output: string;
  };
  start_reindex: { input: { account_id: string | null }; output: string };
  cancel_reindex: { input: { task_id: string }; output: null };
  preview_sync_range: {
    input: { account_id: string; months: number | null };
    output: SyncRangePreview;
  };
  update_sync_range: {
    input: { account_id: string; months: number | null };
    output: number;
  };

  // Legal analysis (T070/T071, dev/02 §Module D) — real backend command.
  analyze_legal_risk: {
    input: { params: AnalyzeLegalRiskParams };
    output: LegalAnalysisResult;
  };

  // Risk events (T071, dev/02 §Module E). Backed by the real Rust commands
  // (commands::risk::{list_risk_events,resolve_risk_event}); the mock layer
  // below is only the off-Tauri dev/test double. Hooks: queries/risk.ts.
  list_risk_events: {
    input: { params: ListRiskEventsParams };
    output: RiskEvent[];
  };
  resolve_risk_event: { input: { params: ResolveRiskParams }; output: null };

  // Mail list (G2/G3). Backed by the real Rust commands (commands::mail::{
  // list_threads, list_mails, get_mail, set_mail_read, set_mail_starred,
  // archive_mail, delete_mail}, registered in lib.rs); the mock layer below is
  // only the off-Tauri dev/test double. The hook surface (T036) is final.
  list_threads: {
    input: { params: ListThreadsParams };
    output: PageResult<Thread>;
  };
  list_mails: {
    input: { params: ListMailsParams };
    output: PageResult<MailSummary>;
  };
  get_mail: { input: { mail_id: string }; output: MailDetail };
  set_mail_read: { input: { mail_id: string; is_read: boolean }; output: null };
  set_mail_starred: {
    input: { mail_id: string; is_starred: boolean };
    output: null;
  };
  archive_mail: { input: { mail_id: string }; output: null };
  delete_mail: { input: { mail_id: string }; output: null };
};

export type CommandName = keyof Commands;

/** True when running inside the Tauri webview (vs. a plain `pnpm dev` browser). */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const SAMPLE_ACCOUNT: Account = {
  id: "demo-1",
  email: "you@example.com",
  displayName: "Work",
  provider: "imap",
  imapHost: "imap.example.com",
  imapPort: 993,
  smtpHost: "smtp.example.com",
  smtpPort: 587,
  colorToken: "slate",
  badgeLabel: "W",
  roleType: "work",
  roleDescription: null,
  authLevel: 1,
  isPrimary: true,
  isActive: true,
  syncIntervalSecs: 60,
  lastSyncedAt: null,
  knowledgeDepthMonths: 12,
  createdAt: 0,
  updatedAt: 0,
};

// ── Dev/browser fixtures for the mail-list + search surfaces (07 §11) ─────────
const NOW = Math.floor(Date.now() / 1000);

const SAMPLE_THREADS: Thread[] = [
  {
    id: "t-1",
    accountId: "demo-1",
    subject: "Q4 budget review — final numbers",
    participants: ["alice@northwind.co", "you@example.com"],
    mailCount: 4,
    unreadCount: 1,
    hasAttachments: true,
    latestDate: NOW - 1800,
    snippet:
      "The revised figures are attached. Can you confirm the marketing line item before Friday?",
    isArchived: false,
    isStarred: true,
  },
  {
    id: "t-2",
    accountId: "demo-1",
    subject: "Re: Vendor contract renewal",
    participants: ["legal@brightpath.com", "you@example.com"],
    mailCount: 2,
    unreadCount: 0,
    hasAttachments: false,
    latestDate: NOW - 7200,
    snippet: "Counsel flagged clause 7.2 on liability. Suggest we push back before signing.",
    isArchived: false,
    isStarred: false,
  },
  {
    id: "t-3",
    accountId: "demo-1",
    subject: "Lunch Thursday?",
    participants: ["sam@example.com", "you@example.com"],
    mailCount: 3,
    unreadCount: 2,
    hasAttachments: false,
    latestDate: NOW - 86400,
    snippet: "The new place on 5th has great reviews — works for me at 12:30.",
    isArchived: false,
    isStarred: false,
  },
];

const SAMPLE_MAILS: MailSummary[] = SAMPLE_THREADS.map((t, i) => ({
  id: `m-${i + 1}`,
  accountId: t.accountId,
  threadId: t.id,
  subject: t.subject,
  fromName: t.participants[0]?.split("@")[0] ?? null,
  fromEmail: t.participants[0] ?? "unknown@example.com",
  snippet: t.snippet,
  dateSent: t.latestDate,
  isRead: t.unreadCount === 0,
  hasAttachments: t.hasAttachments,
}));

const SAMPLE_DETAIL: MailDetail = {
  id: "m-1",
  accountId: "demo-1",
  threadId: "t-1",
  subject: "Q4 budget review — final numbers",
  fromName: "Alice Nguyen",
  fromEmail: "alice@northwind.co",
  to: [{ name: "You", email: "you@example.com" }],
  cc: [],
  dateSent: NOW - 1800,
  bodyHtml:
    "<p>Hi,</p><p>The revised Q4 figures are attached. Can you confirm the marketing line item before Friday so we can lock the board deck?</p><p>Thanks,<br>Alice</p>",
  bodyText:
    "Hi,\n\nThe revised Q4 figures are attached. Can you confirm the marketing line item before Friday so we can lock the board deck?\n\nThanks,\nAlice",
  isRead: false,
  isStarred: true,
  isArchived: false,
  hasAttachments: true,
  folder: "INBOX",
};

const SAMPLE_SEARCH_RESULTS: SearchResult[] = [
  {
    mailId: "m-1",
    accountId: "demo-1",
    subject: "Q4 budget review — final numbers",
    fromName: "Alice Nguyen",
    fromEmail: "alice@northwind.co",
    dateSent: NOW - 1800,
    snippet: "The revised Q4 budget figures are attached…",
    score: 0.86,
    scoreLabel: "high",
    highlights: ["The revised Q4 <mark>budget</mark> figures are attached"],
  },
  {
    mailId: "m-2",
    accountId: "demo-1",
    subject: "Re: Vendor contract renewal",
    fromName: "Brightpath Legal",
    fromEmail: "legal@brightpath.com",
    dateSent: NOW - 7200,
    snippet: "Counsel flagged clause 7.2 on liability…",
    score: 0.52,
    scoreLabel: "mid",
    highlights: [],
  },
];

// Attachment-origin search hits for the dev/browser search panel (T110).
const SAMPLE_ATTACHMENT_HITS: AttachmentHit[] = [
  {
    attachmentId: "att-1",
    mailId: "m-1",
    filename: "Q4-budget.pdf",
    contentType: "application/pdf",
    excerpt: "…the revised <mark>budget</mark> figures for Q4 are summarised on page two…",
    score: 0.74,
    mailSubject: "Q4 budget review — final numbers",
    mailFromEmail: "alice@northwind.co",
    mailDateSent: NOW - 1800,
  },
];

const SAMPLE_SAVED_SEARCH: SavedSearch = {
  id: "s-1",
  accountId: null,
  name: "Unpaid invoices",
  query: "invoice unpaid",
  mode: "semantic",
  sortOrder: 0,
  createdAt: NOW - 200000,
};

const SAMPLE_SAVED_SEARCHES: SavedSearch[] = [
  SAMPLE_SAVED_SEARCH,
  {
    id: "s-2",
    accountId: null,
    name: "Contracts to review",
    query: "contract review has:attachment",
    mode: "keyword",
    sortOrder: 1,
    createdAt: NOW - 100000,
  },
];

/**
 * Off-Tauri AI-draft store (Module E): stateful rows so the Pending review
 * panel and the E1 manual-reply flow behave like the real `ai_drafts` table in
 * dev and tests. One E2 semi-auto draft ships as a fixture so /pending renders.
 */
const MOCK_AI_DRAFTS: AiDraft[] = [
  {
    id: "ai-draft-1",
    triggerMailId: "m-1",
    accountId: "demo-1",
    toAddr: { name: "Alice Nguyen", email: "alice@northwind.co" },
    ccAddrs: [],
    subject: "Re: Q4 budget review — final numbers",
    bodyOriginal:
      "Hi Alice,\n\nThanks for sending the revised figures. The marketing line item looks right to me — confirmed for the board deck.\n\nBest,\nYou",
    bodyCurrent:
      "Hi Alice,\n\nThanks for sending the revised figures. The marketing line item looks right to me — confirmed for the board deck.\n\nBest,\nYou",
    isEdited: false,
    styleMatchScore: 0.92,
    triggerMode: "E2_semi",
    aiModel: "mock-model",
    knowledgeRefs: ["m-2"],
    status: "pending",
    sendAfter: null,
    expiresAt: NOW + 72 * 3600,
    sentAt: null,
    discardedAt: null,
    discardReason: null,
    createdAt: NOW - 900,
    updatedAt: NOW - 900,
  },
];

let mockAiDraftSeq = 1;

function mockNewAiDraft(triggerMailId: string, triggerMode: AiDraft["triggerMode"]): AiDraft {
  mockAiDraftSeq += 1;
  const now = Math.floor(Date.now() / 1000);
  const draft: AiDraft = {
    id: `ai-draft-${mockAiDraftSeq}`,
    triggerMailId,
    accountId: SAMPLE_DETAIL.accountId,
    toAddr: { name: SAMPLE_DETAIL.fromName, email: SAMPLE_DETAIL.fromEmail },
    ccAddrs: [],
    subject: `Re: ${SAMPLE_DETAIL.subject}`,
    bodyOriginal:
      "Hi Alice,\n\nConfirming the marketing line item — the revised numbers work for the board deck. I'll have the sign-off to you before Friday.\n\nBest,\nYou",
    bodyCurrent:
      "Hi Alice,\n\nConfirming the marketing line item — the revised numbers work for the board deck. I'll have the sign-off to you before Friday.\n\nBest,\nYou",
    isEdited: false,
    styleMatchScore: 0.88,
    triggerMode,
    aiModel: "mock-model",
    knowledgeRefs: [],
    status: "pending",
    sendAfter: null,
    expiresAt: now + 72 * 3600,
    sentAt: null,
    discardedAt: null,
    discardReason: null,
    createdAt: now,
    updatedAt: now,
  };
  MOCK_AI_DRAFTS.push(draft);
  return draft;
}

function mockFindAiDraft(id: string): AiDraft {
  const draft = MOCK_AI_DRAFTS.find((d) => d.id === id);
  if (!draft) throw { code: "NOT_FOUND", message: "AI draft not found." };
  return draft;
}

/**
 * Off-Tauri E7 audit fixtures (T089): a small `ai_decisions` slice so the
 * Report → AI Activity tab renders rows, summary numbers, and the mis-send
 * flow in dev and tests. Shapes mirror the T088 wire contract (the generated
 * `AiDecisionRow` in `@shared/bindings`).
 */
const MOCK_AI_DECISIONS: AiDecisionRow[] = [
  {
    id: "dec-0001-aaaaaa",
    accountId: "demo-1",
    mailId: "m-1",
    draftId: "ai-draft-1",
    decisionType: "draft_created",
    impact: "draft",
    actionDescription: "Generated a semi-auto reply draft for review.",
    resultDescription: "Draft queued in the Pending review list.",
    knowledgeRefs: ["m-2"],
    knowledgeSummary: "Prior budget thread with the same counterpart.",
    aiModel: "mock-model",
    inputTokens: 1240,
    outputTokens: 310,
    latencyMs: 2150,
    createdAt: NOW - 5 * 3600,
    mailSubject: "Q4 budget review — final numbers",
  },
  {
    id: "dec-0002-bbbbbb",
    accountId: "demo-1",
    mailId: "m-1",
    draftId: "ai-draft-1",
    decisionType: "draft_sent",
    impact: "outbound",
    actionDescription: "Sent the approved reply draft.",
    resultDescription: "Reply delivered to the counterpart.",
    knowledgeRefs: [],
    knowledgeSummary: null,
    aiModel: "mock-model",
    inputTokens: null,
    outputTokens: null,
    latencyMs: 480,
    createdAt: NOW - 4 * 3600,
    mailSubject: "Q4 budget review — final numbers",
  },
  {
    id: "dec-0003-cccccc",
    accountId: "demo-1",
    mailId: "m-2",
    draftId: null,
    decisionType: "risk_intercepted",
    impact: "blocked",
    actionDescription: "Flagged an auto-renewal clause as high risk (T4).",
    resultDescription: "Reply held until the risk warning is resolved.",
    knowledgeRefs: ["m-1"],
    knowledgeSummary: "Earlier contract revision in the same thread.",
    aiModel: "mock-model",
    inputTokens: 890,
    outputTokens: 120,
    latencyMs: 1320,
    createdAt: NOW - 3 * 3600,
    mailSubject: "Re: Vendor contract renewal",
  },
  {
    id: "dec-0004-dddddd",
    accountId: "demo-1",
    mailId: "m-3",
    draftId: "ai-draft-2",
    decisionType: "e3_auto_sent",
    impact: "outbound",
    actionDescription: "Auto-replied to a whitelisted contact.",
    resultDescription: "Reply sent without human review (Full Auto).",
    knowledgeRefs: [],
    knowledgeSummary: null,
    aiModel: "mock-model",
    inputTokens: 1010,
    outputTokens: 280,
    latencyMs: 1980,
    createdAt: NOW - 2 * 3600,
    mailSubject: "Lunch Thursday?",
  },
  {
    id: "dec-0005-eeeeee",
    accountId: "demo-1",
    mailId: null,
    draftId: null,
    decisionType: "trust_downgraded",
    impact: "policy",
    actionDescription: "Demoted the account to Semi-Auto after mis-send reports.",
    resultDescription: "Authorization level lowered; drafts now require review.",
    knowledgeRefs: [],
    knowledgeSummary: null,
    aiModel: null,
    inputTokens: null,
    outputTokens: null,
    latencyMs: null,
    createdAt: NOW - 3600,
    mailSubject: null,
  },
];

/** Mirrors the T088 summary aggregation over a decision slice. */
function mockDecisionSummary(rows: AiDecisionRow[]): DecisionSummary {
  const count = (type: string) => rows.filter((r) => r.decisionType === type).length;
  const autoSent = count("e3_auto_sent") + count("auto_reply_sent");
  const failed = count("risk_intercepted") + count("e4_sensitive");
  return {
    totalEvents: rows.length,
    autoSentCount: autoSent,
    downgradeCount: count("trust_downgraded"),
    sensitiveCount: count("e4_sensitive"),
    draftSentCount: count("draft_sent"),
    draftCreatedCount: count("draft_created"),
    totalInputTokens: rows.reduce((sum, r) => sum + (r.inputTokens ?? 0), 0),
    totalOutputTokens: rows.reduce((sum, r) => sum + (r.outputTokens ?? 0), 0),
    successRate: rows.length === 0 ? 1 : (rows.length - failed) / rows.length,
  };
}

function mockFilterDecisions(params: {
  accountId?: string | null;
  sinceUnix?: number | null;
  untilUnix?: number | null;
  decisionTypes?: string[] | null;
}): AiDecisionRow[] {
  return MOCK_AI_DECISIONS.filter(
    (r) =>
      (params.accountId == null || r.accountId === params.accountId) &&
      (params.sinceUnix == null || r.createdAt >= params.sinceUnix) &&
      (params.untilUnix == null || r.createdAt <= params.untilUnix) &&
      (params.decisionTypes == null ||
        params.decisionTypes.length === 0 ||
        params.decisionTypes.includes(r.decisionType)),
  );
}

/**
 * Off-Tauri settings store: lets the settings pages behave statefully in a plain
 * browser and in unit tests (the privacy defaults mirror T051 §6).
 */
const MOCK_SETTINGS = new Map<string, string>([
  ["privacy.tracker_policy", JSON.stringify("block_known")],
  ["privacy.remote_image_policy", JSON.stringify("block_all")],
]);

/**
 * Off-Tauri AI-settings store (T073): stateful per-account rows so the /agents
 * page behaves like the real `account_ai_settings` table in dev and tests.
 * Defaults mirror the schema defaults (dev/01 §account_ai_settings).
 */
const MOCK_AI_SETTINGS = new Map<string, AccountAiSettings>();

/**
 * Off-Tauri F3 setup state (T064): the disclosure / quota / first-auth stamps
 * behave statefully so the setup wizard is exercisable in dev and tests.
 */
const MOCK_AI_SETUP: AiSetupStatus = {
  disclosureConfirmedAt: null,
  conservativeQuotaUntil: null,
  firstAuthAt: null,
};

/** Mirrors the Rust `RECOMMENDED_PROVIDERS` config (T064, F_F3 §4.1). */
const MOCK_BALANCED_PROVIDER: RecommendedProviderInfo = {
  tier: "balanced",
  provider: "anthropic",
  model: "claude-sonnet-4-5",
  displayName: "Anthropic Claude (balanced)",
  monthlyCostMinUsd: 3,
  monthlyCostMaxUsd: 9,
  tokensPerReplyEstimate: 1500,
  oauthSupported: true,
};

const MOCK_RECOMMENDED_PROVIDERS: RecommendedProviderInfo[] = [
  MOCK_BALANCED_PROVIDER,
  {
    tier: "high_quality",
    provider: "openai",
    model: "gpt-5",
    displayName: "OpenAI flagship (high quality)",
    monthlyCostMinUsd: 12,
    monthlyCostMaxUsd: 30,
    tokensPerReplyEstimate: 1800,
    oauthSupported: true,
  },
];

function mockAiSettingsRow(accountId: string): AccountAiSettings {
  const existing = MOCK_AI_SETTINGS.get(accountId);
  if (existing) return existing;
  const fresh: AccountAiSettings = {
    accountId,
    authLevel: 1,
    aiProvider: "none",
    aiModel: null,
    aiBaseUrl: null,
    t1Enabled: true,
    t2Enabled: true,
    t3Enabled: true,
    t4Enabled: true,
    t5Enabled: false,
    t6Enabled: true,
    dailyQueryLimit: 10,
    e3WhitelistOnly: true,
    e3MinHistory: 3,
    styleSamplesCount: 0,
    updatedAt: NOW,
  };
  MOCK_AI_SETTINGS.set(accountId, fresh);
  return fresh;
}

/**
 * Off-Tauri F4 matrix store (T066): one stateful `CapabilityMatrix` per
 * account. Reads of an unconfigured account return the computed defaults
 * without persisting them, and the default/validation/warning logic mirrors
 * the backend (`src-tauri/src/ai/matrix.rs`: `build_default_matrix`,
 * `CapabilityMatrix::validate`, `CapabilityMatrix::warnings`).
 */
const MOCK_MATRICES = new Map<string, CapabilityMatrix>();

const MOCK_MATRIX_CAPABILITIES: Capability[] = [
  "DraftReply",
  "RiskReason",
  "Summarize",
  "StyleProfile",
];

/** Mirrors `build_default_matrix` over the mock AI-settings rows (F_F4 §4.1). */
function mockDefaultMatrix(accountId: string): CapabilityMatrix {
  const base = mockAiSettingsRow(accountId);
  const localAvailable = [...MOCK_AI_SETTINGS.values()].some(
    (row) => row.aiProvider === "local_onnx",
  );
  const entries: MatrixEntry[] = [];
  for (const capability of MOCK_MATRIX_CAPABILITIES) {
    const prefersLocal = capability === "RiskReason" || capability === "StyleProfile";
    let primary: ProviderAssignment | null = null;
    if (localAvailable && prefersLocal) {
      primary = { provider: "local_onnx", model: "", baseUrl: null };
    } else if (base.aiProvider !== "none") {
      primary = { provider: base.aiProvider, model: base.aiModel ?? "", baseUrl: base.aiBaseUrl };
    }
    if (primary) entries.push({ capability, cell: { primary, backups: [] } });
  }
  return { entries };
}

/** Mirrors `CapabilityMatrix::validate` — throws the wire `VALIDATION` shape. */
function mockValidateMatrix(matrix: CapabilityMatrix): void {
  const seen = new Set<Capability>();
  for (const entry of matrix.entries) {
    if (entry.cell.backups.length > 2) {
      throw { code: "VALIDATION", message: "a matrix cell allows at most 2 backup assignments" };
    }
    for (const backup of entry.cell.backups) {
      if (backup.provider === entry.cell.primary.provider) {
        throw {
          code: "VALIDATION",
          message: `backup provider '${backup.provider}' duplicates the cell's primary provider`,
        };
      }
    }
    if (seen.has(entry.capability)) {
      throw {
        code: "VALIDATION",
        message: `capability '${entry.capability}' is assigned more than once`,
      };
    }
    seen.add(entry.capability);
  }
}

/** Mirrors the Rust `is_small_local_model` "< 7B" heuristic (F_F4 §4.5). */
function mockIsSmallLocalModel(model: string): boolean {
  const size = /(\d+(?:\.\d+)?)b(?![a-z0-9])/i.exec(model)?.[1];
  return size !== undefined && Number.parseFloat(size) < 7;
}

/** Mirrors `CapabilityMatrix::warnings` (advisory, never blocks the save). */
function mockMatrixWarnings(matrix: CapabilityMatrix): MatrixWarning[] {
  const warnings: MatrixWarning[] = [];
  for (const entry of matrix.entries) {
    const primary = entry.cell.primary;
    const isCloud = primary.provider === "openai" || primary.provider === "anthropic";
    const isLocal = primary.provider === "ollama" || primary.provider === "local_onnx";
    if (entry.capability === "Summarize" && isLocal && mockIsSmallLocalModel(primary.model)) {
      warnings.push({
        capability: entry.capability,
        code: "small_local_model",
        message: `Summaries and role audits need strong reasoning; '${primary.model}' looks smaller than 7B and may underperform.`,
      });
    }
    if (entry.capability === "RiskReason" && isCloud) {
      warnings.push({
        capability: entry.capability,
        code: "high_cost_cloud",
        message:
          "Sensitivity checks run on every inbound mail; a cloud model here can add significant cost.",
      });
    }
    if (entry.capability === "StyleProfile" && isCloud) {
      warnings.push({
        capability: entry.capability,
        code: "style_history_to_cloud",
        message:
          "Style learning sends excerpts of your mail history to a public cloud endpoint; consider a local model.",
      });
    }
  }
  return warnings;
}

/** Insert or replace one cell, preserving entry order (immutable). */
function mockSetMatrixCell(
  matrix: CapabilityMatrix,
  capability: Capability,
  cell: MatrixCell,
): CapabilityMatrix {
  const entries = matrix.entries.slice();
  const index = entries.findIndex((e) => e.capability === capability);
  if (index >= 0) entries[index] = { capability, cell };
  else entries.push({ capability, cell });
  return { entries };
}

/**
 * Off-Tauri risk-event store (T071): stateful rows so the T4 banner and the
 * report risk panel behave like the real `risk_events` table in dev and tests
 * (resolve mutates status in place). Shapes mirror dev/01 §risk_events.
 */
const MOCK_RISK_EVENTS: RiskEvent[] = [
  {
    id: "risk-1",
    mailId: "m-1",
    accountId: "demo-1",
    riskLevel: 4,
    riskType: "payment_anomaly",
    evidence: {
      sourceCommand: "analyze_legal_risk",
      excerptSha256: "9f2c41aa",
    },
    description: "Unusually long payment term (net 90) conflicts with the standard net-30 policy.",
    status: "open",
    expiresAt: null,
    createdAt: NOW - 600,
  },
  {
    id: "risk-2",
    mailId: "m-2",
    accountId: "demo-1",
    riskLevel: 3,
    riskType: "rule_conflict",
    evidence: {
      sourceCommand: "analyze_legal_risk",
      excerptSha256: "1b7e90cd",
    },
    description: "Liability clause 7.2 exceeds the negotiated cap from prior agreements.",
    status: "open",
    expiresAt: NOW + 7 * 86400,
    createdAt: NOW - 5400,
  },
];

/** Fixture D1 verdict (T071). `originalText` values occur verbatim in
 *  SAMPLE_DETAIL.bodyHtml so the body-highlight flow works off-Tauri. */
const SAMPLE_LEGAL_ANALYSIS: LegalAnalysisResult = {
  decisionId: "dec-legal-1",
  mailId: "m-1",
  accountId: "demo-1",
  riskList: [
    {
      level: "medium",
      type: "payment",
      originalText: "confirm the marketing line item before Friday",
      finding: "Approval deadline set without a written change order",
      suggestion: "Request a signed change order before confirming the budget",
    },
    {
      level: "high",
      type: "liability",
      originalText: "lock the board deck",
      finding: "Figures become binding once presented to the board",
      suggestion: "Add a draft watermark until the numbers are audited",
    },
    {
      level: "low",
      type: "other",
      originalText: "The revised Q4 figures are attached",
      finding: "Attachment provenance is not verified",
      suggestion: "Confirm the attachment hash with the sender",
    },
  ],
  keyClauses: {
    payment: "Confirmation requested before Friday",
    delivery: null,
    liability: "Board presentation implies sign-off",
    confidentiality: null,
    disputeResolution: null,
  },
  complianceAdvice: [
    "Route budget confirmations through the finance approval workflow.",
    "Keep a written record of the deadline and who set it.",
  ],
  overallLevel: "high",
  aiModel: "mock-model",
  knowledgeRefs: ["m-2"],
  createdAt: NOW - 300,
};

/**
 * Off-Tauri Agent-IM store (T092): stateful rows so the TEAM channel renders and
 * posting works in dev and unit tests. Seeds mirror the real flow — a member-join
 * system message plus one agent status note (content is JSON, English copy).
 */
const MOCK_IM_MESSAGES: ImMessage[] = [
  {
    id: "im-1",
    channelId: "main",
    senderType: "system",
    senderId: "system",
    messageType: "text",
    content: JSON.stringify({ text: "Work (you@example.com) joined the collaboration channel." }),
    linkedEmailId: null,
    status: "resolved",
    createdAt: NOW - 3600,
    readAt: NOW - 3600,
  },
  {
    id: "im-2",
    channelId: "main",
    senderType: "agent",
    senderId: "demo-1",
    messageType: "text",
    content: JSON.stringify({
      text: "Morning sync complete — 3 new threads. Nothing needs your decision yet.",
    }),
    linkedEmailId: null,
    status: "resolved",
    createdAt: NOW - 1800,
    readAt: null,
  },
];

let mockImSeq = MOCK_IM_MESSAGES.length;

/**
 * Off-Tauri proactive-query store (T095/T096): a T1 card and a T4 risk card so the
 * Pending DecisionCard, filter chips, and TEAM badge render in dev and tests.
 * `options` carries the full QA card JSON (the DecisionCard parses it).
 */
const MOCK_PENDING_QUERIES: PendingQuery[] = [
  {
    id: "pq-1",
    accountId: "demo-1",
    mailId: "m-2",
    riskEventId: null,
    triggerType: "T1",
    question: "An unknown sender raised a sensitive topic. Do you recognise them?",
    options: JSON.stringify({
      cardVersion: 1,
      linkedQueryId: "pq-1",
      triggerType: "T1",
      priority: "normal",
      linkedEmailId: "m-2",
      questionText: "An unknown sender raised a sensitive topic. Do you recognise them?",
      options: [
        { id: "opt_known", label: "Yes, I know them", value: "known" },
        { id: "opt_unknown", label: "No, treat as unknown", value: "unknown" },
        { id: "opt_skip", label: "Skip", value: "__skip__" },
      ],
      multiSelect: false,
      freeTextPlaceholder: "Add a note (optional)",
      subQuestions: [],
      response: null,
    }),
    answer: null,
    status: "pending",
    priority: 3,
    expiresAt: NOW + 72 * 3600,
    answeredAt: null,
    createdAt: NOW - 300,
  },
  {
    id: "pq-2",
    accountId: "demo-1",
    mailId: "m-2",
    riskEventId: "risk-1",
    triggerType: "T4",
    question: "A high-risk item was flagged. Confirm how the agent should proceed.",
    options: JSON.stringify({
      cardVersion: 1,
      linkedQueryId: "pq-2",
      triggerType: "T4",
      priority: "high",
      linkedEmailId: "m-2",
      questionText: "A high-risk item was flagged. Confirm how the agent should proceed.",
      options: [
        { id: "opt_confirm", label: "Confirm and proceed", value: "confirm" },
        { id: "opt_block", label: "Block this email", value: "block" },
        { id: "opt_skip", label: "Skip", value: "__skip__" },
        { id: "opt_view_email", label: "View original email", value: "__view_email__" },
      ],
      multiSelect: false,
      freeTextPlaceholder: "Add a note (optional)",
      subQuestions: [],
      response: null,
    }),
    answer: null,
    status: "pending",
    priority: 1,
    expiresAt: null,
    answeredAt: null,
    createdAt: NOW - 120,
  },
];

/**
 * Off-Tauri cloud model catalog (T068): mirrors `GET /v1/models` so the
 * add-cloud-provider "Load models" picker renders in dev and unit tests. The
 * ids are representative current models per provider family.
 */
function mockCloudModels(provider: AiProvider): string[] {
  if (provider === "anthropic") {
    return ["claude-haiku-4-5", "claude-opus-4-8", "claude-sonnet-4-6"];
  }
  return ["gpt-4o", "gpt-5.4", "gpt-5.4-mini", "gpt-5.5"];
}

/**
 * Browser/dev + test fixtures (07 §11). When not under Tauri, `ipc()` resolves
 * from here so the UI renders in a plain browser and unit tests need no runtime.
 */
const MOCK_RESPONSES: {
  [K in CommandName]: (args: Commands[K]["input"]) => Commands[K]["output"];
} = {
  ping: () => ({ message: "pong" }),
  list_accounts: () => [SAMPLE_ACCOUNT],
  get_account: () => SAMPLE_ACCOUNT,
  create_account: () => SAMPLE_ACCOUNT,
  update_account: () => SAMPLE_ACCOUNT,
  delete_account: () => null,
  sign_out_seekermail: () => null,
  get_seekermail_id: () => null,
  set_marketing_consent: () => null,
  begin_google_signin: () => {
    throw new Error("SeekerMail ID sign-in is not available in this build");
  },
  complete_google_signin: () => {
    throw new Error("SeekerMail ID sign-in is not available in this build");
  },
  update_account_password: () => null,
  enable_account: () => ({ ...SAMPLE_ACCOUNT, isActive: true }),
  disable_account: () => ({ ...SAMPLE_ACCOUNT, isActive: false }),
  set_primary_account: (args) => ({ ...SAMPLE_ACCOUNT, id: args.account_id, isPrimary: true }),
  get_account_ai_settings: (args) => mockAiSettingsRow(args.account_id),
  update_account_ai_settings: (args) => {
    const current = mockAiSettingsRow(args.account_id);
    const p = args.params;
    const next: AccountAiSettings = {
      ...current,
      authLevel: p.authLevel ?? current.authLevel,
      aiProvider: p.aiProvider ?? current.aiProvider,
      aiModel: p.aiModel ?? current.aiModel,
      aiBaseUrl: p.aiBaseUrl ?? current.aiBaseUrl,
      t1Enabled: p.t1Enabled ?? current.t1Enabled,
      t2Enabled: p.t2Enabled ?? current.t2Enabled,
      t3Enabled: p.t3Enabled ?? current.t3Enabled,
      t4Enabled: p.t4Enabled ?? current.t4Enabled,
      t5Enabled: p.t5Enabled ?? current.t5Enabled,
      t6Enabled: p.t6Enabled ?? current.t6Enabled,
      dailyQueryLimit: p.dailyQueryLimit ?? current.dailyQueryLimit,
      e3WhitelistOnly: p.e3WhitelistOnly ?? current.e3WhitelistOnly,
      e3MinHistory: p.e3MinHistory ?? current.e3MinHistory,
      updatedAt: Math.floor(Date.now() / 1000),
    };
    MOCK_AI_SETTINGS.set(args.account_id, next);
    return next;
  },
  // BYO-AI provider config (T068): stateful off-Tauri fixtures — the list is
  // derived from MOCK_AI_SETTINGS so an "add provider" flow shows up in it.
  verify_ai_provider: (args) => ({
    ok: true,
    modelName: args.params.model,
    errorMessage: null,
  }),
  scan_local_providers: () => [{ baseUrl: "http://localhost:11434", provider: "ollama" }],
  list_ollama_models: () => [
    { name: "llama3:8b", sizeBytes: 4_661_224_676, parameterSize: "8B", quantization: "Q4_0" },
    {
      name: "qwen2.5:14b",
      sizeBytes: 8_988_124_069,
      parameterSize: "14B",
      quantization: "Q4_K_M",
    },
  ],
  list_cloud_models: (args) => mockCloudModels(args.params.provider),
  list_configured_providers: () =>
    [SAMPLE_ACCOUNT]
      .map((account) => ({ account, ai: mockAiSettingsRow(account.id) }))
      .filter(({ ai }) => ai.aiProvider !== "none")
      .map(({ account, ai }) => ({
        accountId: account.id,
        email: account.email,
        displayName: account.displayName,
        colorToken: account.colorToken,
        provider: ai.aiProvider,
        model: ai.aiModel,
        baseUrl: ai.aiBaseUrl,
        authLevel: ai.authLevel,
        isLocal: ai.aiProvider === "ollama" || ai.aiProvider === "local_onnx",
        available: true,
        updatedAt: ai.updatedAt,
      })),
  verify_account_connection: () => ({
    imapOk: true,
    smtpOk: true,
    errorMessage: null,
  }),
  begin_oauth_flow: () => ({
    authorizeUrl: "https://accounts.example.com/authorize",
    state: "mock-state-nonce",
  }),
  complete_oauth_flow: () => null,
  reauth_account: () => null,
  sample_mailbox: () => ({
    ranges: [
      { months: 3, mailCount: 1200, estimatedMb: 350 },
      { months: 6, mailCount: 2400, estimatedMb: 700 },
      { months: 12, mailCount: 4800, estimatedMb: 1400 },
      { months: 36, mailCount: 9600, estimatedMb: 2800 },
      { months: 60, mailCount: 14400, estimatedMb: 4200 },
      { months: null, mailCount: 18000, estimatedMb: 5200 },
    ],
  }),
  set_knowledge_depth: () => SAMPLE_ACCOUNT,
  get_account_disk_usage: () => ({
    totalBytes: 0,
    attachmentBytes: 0,
    bodyBytes: 0,
  }),
  trigger_sync: () => null,
  get_sync_state: () => ({
    accountId: "demo-1",
    lastSyncAt: null,
    lastSyncResult: "ok",
    consecutiveErrors: 0,
    backoffUntil: null,
    inboxUidValidity: 1,
    inboxUidNext: 1,
    fullSyncRequired: false,
    totalMailsSynced: 0,
    updatedAt: 0,
  }),
  get_backfill_status: () => ({
    accountId: "demo-1",
    status: "idle",
    depthMonths: 12,
    boundaryDate: null,
    lastUidFetched: null,
    totalUidCount: null,
    fetchedCount: 0,
    startedAt: null,
    pausedAt: null,
    completedAt: null,
    errorMessage: null,
    updatedAt: 0,
  }),
  pause_backfill: () => null,
  resume_backfill: () => null,
  download_attachment: () => "demo/attachments/2026/06/m/file.pdf",
  get_attachments_for_mail: () => [],
  open_attachment: () => null,
  reveal_attachment: () => null,
  get_attachment_local_path: () => null,
  open_external_url: () => null,
  get_tracker_info: () => ({
    blocked: false,
    trackerCount: 0,
    imagesAllowed: false,
    senderEmail: "sender@example.com",
  }),
  allow_remote_images: () => null,

  // Search
  keyword_search: () => ({
    items: SAMPLE_SEARCH_RESULTS,
    total: SAMPLE_SEARCH_RESULTS.length,
    offset: 0,
  }),
  semantic_search: () => ({
    items: SAMPLE_SEARCH_RESULTS,
    total: SAMPLE_SEARCH_RESULTS.length,
    offset: 0,
  }),
  get_search_history: () => [
    {
      id: 2,
      query: "budget review",
      mode: "semantic",
      resultCount: 5,
      createdAt: NOW - 3600,
    },
    {
      id: 1,
      query: "from:alice",
      mode: "keyword",
      resultCount: 12,
      createdAt: NOW - 9000,
    },
  ],
  list_saved_searches: () => SAMPLE_SAVED_SEARCHES,
  save_search: () => SAMPLE_SAVED_SEARCH,
  delete_saved_search: () => null,
  get_gte_stats: () => ({
    emailCount: 91450,
    indexedCount: 87230,
    unindexedCount: 4220,
    queuePending: 12,
    spamExcluded: 3420,
    vectorCount: 87230,
    coveragePct: 95.4,
    model: "bge-m3",
    dimensions: 1024,
    indexVersion: "v47",
    storageBytes: 134_217_728,
    usedToday: 7,
    risksCaught: 3,
    accountsSyncing: 3,
    lastSyncAt: NOW - 120,
  }),
  // Top Topics now groups the AI decision log by its `impact` class
  // (risk | reply | identity | rule | context) — see commands/gte.rs.
  get_topic_breakdown: () => [
    { label: "Replies", color: "green", count: 84 },
    { label: "Context", color: "sage", count: 52 },
    { label: "Risk", color: "terra", count: 31 },
    { label: "Identity", color: "slate", count: 19 },
    { label: "Rules", color: "amber", count: 12 },
  ],
  list_knowledge_entries: () => [
    {
      id: "k1",
      accountId: "demo-1",
      acctColor: "terra",
      acctBadge: "L",
      subject: "Q3 Service Contract — Non-Compete Clause Analysis",
      excerpt:
        "boss@corp.com requested adding a non-compete restriction in Clause 12, barring Party B from working in similar industries for two years post-contract — exceeds standard NDA scope.",
      body: "<p>Key findings:</p><p>The counterparty added a 24-month non-compete covering “same or similar industries.” Recommendation: reject; retain Clause 12 standard confidentiality terms.</p>",
      tags: ["Contract", "NDA", "Compliance"],
      dateSent: NOW - 3 * 3600,
      usedCount: 4,
      impact: "rule",
      lastUsedFor:
        "AI identified non-compete clause as exceeding standard scope; drafted negotiation reply recommending rejection",
      lastUsedTime: NOW - 1800,
      source: "boss@corp.com",
      thread: "Q3 Service Contract Renewal",
      indexedAt: NOW - 4 * 3600,
    },
    {
      id: "k2",
      accountId: "demo-1",
      acctColor: "slate",
      acctBadge: "W",
      subject: "Vendor Inc. Payment History",
      excerpt:
        "14 payments totalling ¥612,000 over 18 months, all from @vendor.com. Anomaly: @vendor.io appeared on 2026-04-23.",
      body: "<p>Vendor Inc. payment summary: 14 payments · ¥612,000 over 18 months, historical domain @vendor.com.</p><p>⚠ Anomaly: ¥48,000 request from @vendor.io — domain mismatch, T4 risk alert.</p>",
      tags: ["Payment", "Vendor"],
      dateSent: NOW - 5 * 3600,
      usedCount: 3,
      impact: "risk",
      lastUsedFor:
        "Detected @vendor.io ≠ @vendor.com — triggered T4 risk alert and paused automatic processing",
      lastUsedTime: NOW - 2 * 3600,
      source: "ap@vendor.com",
      thread: "Payment Confirmation Thread",
      indexedAt: NOW - 5 * 3600,
    },
    {
      id: "k3",
      accountId: "demo-1",
      acctColor: "terra",
      acctBadge: "L",
      subject: "Standard NDA Template v2.3",
      excerpt:
        "Company NDA template, default validity 24 months; non-compete exemption clause included.",
      body: "<p>NDA template: 24-month validity (extendable to 36), non-compete not included by default.</p>",
      tags: ["NDA", "Contract"],
      dateSent: NOW - 30 * 86400,
      usedCount: 6,
      impact: "rule",
      lastUsedFor:
        "Compared counterparty clause against standard NDA — confirmed clause exceeds scope",
      lastUsedTime: NOW - 1800,
      source: "Internal document",
      thread: "Legal Template Library",
      indexedAt: NOW - 30 * 86400,
    },
    {
      id: "k4",
      accountId: "demo-1",
      acctColor: "slate",
      acctBadge: "W",
      subject: "Vendor Payment Risk — Domain Anomaly",
      excerpt:
        "@vendor.io detected in a payment request; all history shows @vendor.com. GTE flagged T4 high-risk.",
      body: "<p>Risk event: sender ap@vendor.io (anomalous) vs historical ap@vendor.com. Amount ¥48,000. Decline until phone confirmation.</p>",
      tags: ["Payment", "Compliance", "Vendor"],
      dateSent: NOW - 4 * 3600,
      usedCount: 3,
      impact: "risk",
      lastUsedFor:
        "Provided evidence basis for AI T4 alert card — required manual confirmation before processing",
      lastUsedTime: NOW - 2 * 3600,
      source: "ap@vendor.io",
      thread: "Payment Request Risk Record",
      indexedAt: NOW - 4 * 3600,
    },
    {
      id: "k5",
      accountId: "demo-1",
      acctColor: "sage",
      acctBadge: "P",
      subject: "Contact Profile — boss@corp.com",
      excerpt:
        "23 historical emails. Decision style favours written confirmation; replies typically weekday mornings 9–11 AM.",
      body: "<p>Contact profile: formal style, requires clear action items, reply window weekdays 09:00–11:00.</p>",
      tags: ["Vendor"],
      dateSent: NOW - 6 * 3600,
      usedCount: 7,
      impact: "identity",
      lastUsedFor: "AI recognised familiar contact; generated reply matching their formal style",
      lastUsedTime: NOW - 1800,
      source: "boss@corp.com",
      thread: "Contact Profile",
      indexedAt: NOW - 6 * 3600,
    },
    {
      id: "k6",
      accountId: "demo-1",
      acctColor: "terra",
      acctBadge: "L",
      subject: "Annual Compliance Checklist 2026",
      excerpt:
        "Regulatory review items: data privacy, contract archiving, vendor credential audit — all due by 2026-06-30.",
      body: "<p>2026 compliance items: PIPL assessment (due 2026-05-31), contract archiving (2026-06-30), vendor credential re-audit (2026-06-15).</p>",
      tags: ["Compliance", "Contract"],
      dateSent: NOW - 2 * 86400,
      usedCount: 1,
      impact: "context",
      lastUsedFor: "AI generated a structured compliance progress reply",
      lastUsedTime: NOW - 86400,
      source: "compliance@internal",
      thread: "Annual Compliance Plan",
      indexedAt: NOW - 2 * 86400,
    },
  ],
  search_with_attachments: () => ({
    mailHits: SAMPLE_SEARCH_RESULTS,
    attachmentHits: SAMPLE_ATTACHMENT_HITS,
  }),
  start_attachment_extraction_backfill: () => ({ pendingCount: 0 }),
  build_attachment_index: () => ({ totalPending: 0, started: true }),

  // Compose / send
  send_mail: () => ({
    pendingId: "pending-1",
    messageId: "<pending-1@seekermail.local>",
  }),
  cancel_send: () => ({ cancelled: true }),

  // Drafts
  save_draft: () => ({
    id: "draft-1",
    accountId: "demo-1",
    to: [],
    cc: [],
    subject: "",
    bodyText: "",
    bodyHtml: null,
    inReplyTo: null,
    updatedAt: NOW,
  }),
  get_draft: () => ({
    id: "draft-1",
    accountId: "demo-1",
    to: [],
    cc: [],
    subject: "",
    bodyText: "",
    bodyHtml: null,
    inReplyTo: null,
    updatedAt: NOW,
  }),
  delete_draft: () => null,

  // AI drafts (Module E) — stateful: mutations update MOCK_AI_DRAFTS in place.
  request_ai_reply: (args) => mockNewAiDraft(args.params.mailId, "E1_manual"),
  regenerate_draft: (args) => {
    const draft = mockFindAiDraft(args.params.id);
    draft.bodyOriginal =
      "Hi Alice,\n\nThe revised Q4 figures look good — consider the marketing line item confirmed. Happy to walk the board through the changes if useful.\n\nBest,\nYou";
    draft.bodyCurrent = draft.bodyOriginal;
    draft.isEdited = false;
    draft.status = "pending";
    draft.updatedAt = Math.floor(Date.now() / 1000);
    return { ...draft };
  },
  list_pending_drafts: (args) =>
    MOCK_AI_DRAFTS.filter(
      (d) =>
        (d.status === "pending" || d.status === "edited") &&
        (args.params.accountId === null || d.accountId === args.params.accountId),
    ).map((d) => ({ ...d })),
  get_ai_draft: (args) => ({ ...mockFindAiDraft(args.id) }),
  update_draft_body: (args) => {
    const draft = mockFindAiDraft(args.id);
    draft.bodyCurrent = args.body_current;
    draft.isEdited = draft.bodyCurrent !== draft.bodyOriginal;
    if (draft.status === "pending" && draft.isEdited) draft.status = "edited";
    draft.updatedAt = Math.floor(Date.now() / 1000);
    return { ...draft };
  },
  approve_draft: (args) => {
    const draft = mockFindAiDraft(args.id);
    const now = Math.floor(Date.now() / 1000);
    draft.status = "sent";
    draft.sentAt = now;
    draft.updatedAt = now;
    return {
      sentAt: now,
      messageId: `<${draft.id}@seekermail.local>`,
      pendingId: `pending-${draft.id}`,
    };
  },
  discard_draft: (args) => {
    const draft = mockFindAiDraft(args.id);
    const now = Math.floor(Date.now() / 1000);
    draft.status = "discarded";
    draft.discardedAt = now;
    draft.discardReason = args.reason;
    draft.updatedAt = now;
    return null;
  },
  cancel_draft_send: (args) => {
    const draft = mockFindAiDraft(args.id);
    if (draft.status === "sent") {
      throw { code: "CONFLICT", message: "Draft already sent; cannot cancel." };
    }
    // Still pending/edited → approve_draft has not run yet; no-op is correct.
    return { ...draft };
  },

  // E7 audit log (T088/T089)
  list_ai_decisions: (args) => {
    const rows = mockFilterDecisions(args.params)
      .slice()
      .sort((a, b) => b.createdAt - a.createdAt);
    const offset = args.params.offset ?? 0;
    const limit = args.params.limit ?? rows.length;
    return rows.slice(offset, offset + limit).map((r) => ({ ...r }));
  },
  get_ai_decisions_summary: (args) =>
    mockDecisionSummary(
      mockFilterDecisions({
        accountId: args.accountId,
        sinceUnix: args.sinceUnix,
        untilUnix: args.untilUnix,
      }),
    ),
  export_ai_decisions: (args) =>
    `demo/exports/ai_decisions_${new Date(NOW * 1000).toISOString().slice(0, 10)}.${args.params.format}`,

  // Settings (T050/T051)
  get_setting: (args) => MOCK_SETTINGS.get(args.key) ?? null,
  set_setting: (args) => {
    MOCK_SETTINGS.set(args.key, args.value);
    return null;
  },
  set_ai_disabled: (args) => {
    // Mirror the Rust command: a deadline writes the raw integer string the
    // fallback router reads; null deletes the key (AI restored).
    if (args.until === null) MOCK_SETTINGS.delete("ai.disable_until");
    else MOCK_SETTINGS.set("ai.disable_until", String(args.until));
    return null;
  },
  apply_privacy_policy: (args) => {
    MOCK_SETTINGS.set("privacy.tracker_policy", JSON.stringify(args.tracker_policy));
    MOCK_SETTINGS.set("privacy.remote_image_policy", JSON.stringify(args.remote_image_policy));
    return null;
  },

  // Export (T052)
  start_export: () => "mock-export-task",
  cancel_export: () => null,
  open_export_output: () => null,

  // Wipe / reindex / sync range (T053)
  preview_wipe: () => ({
    mailCount: 4800,
    attachmentCount: 120,
    estimatedBytes: 1_400_000_000,
  }),
  start_wipe: () => "mock-wipe-task",
  start_reindex: () => "mock-reindex-task",
  cancel_reindex: () => null,
  preview_sync_range: () => ({ mailsBeyondRange: 230 }),
  update_sync_range: () => 0,

  // F4 provider matrix (T066): stateful per-account store; defaults, cell
  // validation, and advisory warnings all mirror the Rust backend.
  get_provider_matrix: (args) =>
    MOCK_MATRICES.get(args.account_id) ?? mockDefaultMatrix(args.account_id),
  update_provider_matrix: (args) => {
    mockValidateMatrix(args.matrix);
    MOCK_MATRICES.set(args.account_id, args.matrix);
    return mockMatrixWarnings(args.matrix);
  },
  reset_provider_matrix_to_defaults: (args) => {
    const matrix = mockDefaultMatrix(args.account_id);
    MOCK_MATRICES.set(args.account_id, matrix);
    return matrix;
  },
  batch_update_provider_matrix: (args) => {
    // Stage every write, then validate all matrices before any commit —
    // mirrors `do_batch_update_matrix` (all-or-nothing).
    const staged = new Map<string, CapabilityMatrix>();
    for (const update of args.updates) {
      const current =
        staged.get(update.accountId) ??
        MOCK_MATRICES.get(update.accountId) ??
        mockDefaultMatrix(update.accountId);
      staged.set(update.accountId, mockSetMatrixCell(current, update.capability, update.cell));
    }
    for (const matrix of staged.values()) mockValidateMatrix(matrix);
    for (const [accountId, matrix] of staged) MOCK_MATRICES.set(accountId, matrix);
    return null;
  },

  // Data-flow disclosure (T069) — mirrors the off-Tauri AI-settings default
  // (provider "none"): the demo account discloses no AI endpoint.
  get_data_flow_ai_routing: () => ({
    routes: [
      {
        accountId: SAMPLE_ACCOUNT.id,
        accountEmail: SAMPLE_ACCOUNT.email,
        colorToken: SAMPLE_ACCOUNT.colorToken,
        aiProvider: "none" as const,
        aiModel: null,
        endpointKind: "none" as const,
        endpointUrl: null,
        endpointHost: null,
        isLocal: false,
      },
    ],
    activity: [],
    sinceUnix: NOW - 86_400,
  }),

  // Agent-IM / TEAM channel (T092): stateful store; post appends, list paginates.
  post_im_message: (args) => {
    if (args.channel_id !== "main") {
      throw { code: "VALIDATION", message: "channel_id must be 'main' (no private chats)." };
    }
    mockImSeq += 1;
    const now = Math.floor(Date.now() / 1000);
    const msg: ImMessage = {
      id: `im-${mockImSeq}`,
      channelId: args.channel_id,
      senderType: args.sender_type as ImMessage["senderType"],
      senderId: args.sender_id,
      messageType: args.message_type as ImMessage["messageType"],
      content: args.content,
      linkedEmailId: args.linked_email_id,
      status: "resolved",
      createdAt: now,
      readAt: null,
    };
    MOCK_IM_MESSAGES.push(msg);
    return { ...msg };
  },
  list_im_messages: (args) => {
    const filtered = MOCK_IM_MESSAGES.filter(
      (m) =>
        (args.sender_id == null || m.senderId === args.sender_id) &&
        (args.status == null || m.status === args.status),
    ).sort((a, b) => a.createdAt - b.createdAt);
    const offset = args.offset ?? 0;
    const limit = Math.min(args.limit ?? 50, 200);
    return {
      items: filtered.slice(offset, offset + limit).map((m) => ({ ...m })),
      total: filtered.length,
      offset,
    };
  },
  mark_im_message_read: (args) => {
    const msg = MOCK_IM_MESSAGES.find((m) => m.id === args.id);
    if (msg && msg.readAt === null) msg.readAt = Math.floor(Date.now() / 1000);
    return null;
  },
  mark_im_channel_read: (args) => {
    if (args.channel_id !== "main") {
      throw { code: "VALIDATION", message: "channel_id must be 'main' (no private chats)." };
    }
    const now = Math.floor(Date.now() / 1000);
    for (const m of MOCK_IM_MESSAGES) if (m.readAt === null) m.readAt = now;
    return null;
  },
  count_pending_queries: () => MOCK_PENDING_QUERIES.filter((q) => q.status === "pending").length,
  // Hybrid badge: unread agent messages + still-pending decision cards.
  count_team_unread: () =>
    MOCK_IM_MESSAGES.filter(
      (m) => m.status === "pending" || (m.senderType === "agent" && m.readAt === null),
    ).length,

  // Agent presence (T094): one idle status per known account.
  get_agent_statuses: () => [{ accountId: SAMPLE_ACCOUNT.id, status: "idle" as const }],

  // Proactive queries (T096/T099): stateful store; answer/skip mutate in place.
  list_pending_queries: (args) =>
    MOCK_PENDING_QUERIES.filter(
      (q) =>
        q.status === "pending" && (args.account_id === null || q.accountId === args.account_id),
    ).map((q) => ({ ...q })),
  answer_query: (args) => {
    const q = MOCK_PENDING_QUERIES.find((x) => x.id === args.id);
    if (!q) throw { code: "NOT_FOUND", message: "Query not found." };
    if (q.status !== "pending") throw { code: "FORBIDDEN", message: "Query is not pending." };
    q.status = "answered";
    q.answer = args.answer;
    q.answeredAt = Math.floor(Date.now() / 1000);
    return null;
  },
  skip_query: (args) => {
    const q = MOCK_PENDING_QUERIES.find((x) => x.id === args.id);
    if (!q) throw { code: "NOT_FOUND", message: "Query not found." };
    if (q.status !== "pending") throw { code: "FORBIDDEN", message: "Query is not pending." };
    q.status = "skipped";
    q.answeredAt = Math.floor(Date.now() / 1000);
    return null;
  },

  // F3 recommended-provider setup (T064)
  get_recommended_providers: () => MOCK_RECOMMENDED_PROVIDERS,
  get_ai_setup_status: () => ({ ...MOCK_AI_SETUP }),
  confirm_ai_disclosure: () => {
    MOCK_AI_SETUP.disclosureConfirmedAt = Math.floor(Date.now() / 1000);
    return { ...MOCK_AI_SETUP };
  },
  clear_conservative_quota: () => {
    MOCK_AI_SETUP.conservativeQuotaUntil = null;
    return null;
  },
  begin_recommended_oauth: () => ({
    state: "mock-oauth-state",
    authorizeUrl: "https://auth.example.com/authorize",
  }),
  complete_recommended_oauth: (args) => {
    if (args.code === "invalid-code") {
      return {
        ok: false,
        providerName: MOCK_BALANCED_PROVIDER.displayName,
        modelName: null,
        errorMessage: "The provider rejected the authorization code.",
      };
    }
    const now = Math.floor(Date.now() / 1000);
    MOCK_AI_SETUP.firstAuthAt = MOCK_AI_SETUP.firstAuthAt ?? now;
    MOCK_AI_SETUP.conservativeQuotaUntil = now + 7 * 86_400;
    return {
      ok: true,
      providerName: MOCK_BALANCED_PROVIDER.displayName,
      modelName: MOCK_BALANCED_PROVIDER.model,
      errorMessage: null,
    };
  },
  revoke_recommended_provider: () => null,

  // Legal analysis + risk events (T071)
  analyze_legal_risk: (args) => ({
    ...SAMPLE_LEGAL_ANALYSIS,
    mailId: args.params.mailId,
  }),
  list_risk_events: (args) =>
    MOCK_RISK_EVENTS.filter(
      (e) =>
        (args.params.mailId === undefined || e.mailId === args.params.mailId) &&
        (args.params.accountId === undefined || e.accountId === args.params.accountId) &&
        (args.params.riskLevel === undefined || e.riskLevel === args.params.riskLevel) &&
        e.status === (args.params.status ?? "open"),
    ),
  resolve_risk_event: (args) => {
    const event = MOCK_RISK_EVENTS.find((e) => e.id === args.params.id);
    if (event) event.status = args.params.status;
    return null;
  },

  // Mail list (provisional mocks)
  list_threads: () => ({
    items: SAMPLE_THREADS,
    total: SAMPLE_THREADS.length,
    offset: 0,
  }),
  list_mails: () => ({
    items: SAMPLE_MAILS,
    total: SAMPLE_MAILS.length,
    offset: 0,
  }),
  get_mail: () => SAMPLE_DETAIL,
  set_mail_read: () => null,
  set_mail_starred: () => null,
  archive_mail: () => null,
  delete_mail: () => null,
};

/**
 * Normalise anything thrown across the IPC boundary into the wire `IpcError`
 * shape `{ code, message, detail }` (02 §2, 09 §1).
 */
export function normalizeIpcError(e: unknown): IpcError {
  if (
    e !== null &&
    typeof e === "object" &&
    "code" in e &&
    typeof (e as { code: unknown }).code === "string"
  ) {
    const err = e as { code: string; message?: unknown; detail?: unknown };
    return {
      code: err.code as IpcError["code"],
      message: typeof err.message === "string" ? err.message : "Something went wrong.",
      detail: typeof err.detail === "string" ? err.detail : null,
    };
  }
  return {
    code: "INTERNAL",
    message: e instanceof Error ? e.message : "Something went wrong.",
    detail: null,
  };
}

/** snake_case → camelCase for a single key (`mail_id` → `mailId`). */
function toCamelKey(key: string): string {
  return key.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());
}

/**
 * Tauri v2 binds command arguments by the **camelCase** form of each Rust
 * parameter name (Rust `mail_id` ← JS `mailId`). Our `Commands` map and call
 * sites spell the top-level keys in snake_case for readability, so we camelCase
 * the top-level keys here — once, centrally — before handing them to `invoke`.
 * Only the argument *names* are rewritten; nested DTO values (already camelCase)
 * pass through untouched. Without this, commands with a required snake_case
 * argument (e.g. `get_mail`'s `mail_id`) never bind and reject before running.
 */
function camelizeArgKeys(args: unknown): Record<string, unknown> | undefined {
  if (args === null || args === undefined) return undefined;
  if (typeof args !== "object" || Array.isArray(args)) {
    return args as Record<string, unknown>;
  }
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(args as Record<string, unknown>)) {
    out[toCamelKey(key)] = value;
  }
  return out;
}

/**
 * Invoke a backend command with end-to-end typing and normalised errors. The one
 * seam the whole frontend goes through; components never call `invoke` directly.
 */
export async function ipc<K extends CommandName>(
  cmd: K,
  args?: Commands[K]["input"],
): Promise<Commands[K]["output"]> {
  try {
    if (!isTauri()) {
      const mock = MOCK_RESPONSES[cmd];
      return mock(args as Commands[K]["input"]);
    }
    return await invoke<Commands[K]["output"]>(cmd, camelizeArgKeys(args));
  } catch (e) {
    throw normalizeIpcError(e);
  }
}
