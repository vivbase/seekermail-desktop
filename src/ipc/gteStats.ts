// Hand-written DTO mirrors for the GTE stats + topic-breakdown commands
// (commands/gte.rs). Replaced by the generated bindings once the command surface
// is exported via `pnpm gen:types`; the field set matches the Rust `GteStats` /
// `TopicCount` structs (camelCase via serde(rename_all)).

export interface GteStats {
  emailCount: number;
  indexedCount: number;
  unindexedCount: number;
  queuePending: number;
  spamExcluded: number;
  vectorCount: number;
  coveragePct: number;
  model: string;
  dimensions: number;
  indexVersion: string;
  storageBytes: number;
  usedToday: number;
  risksCaught: number;
  accountsSyncing: number;
  lastSyncAt: number | null;
}

export interface TopicCount {
  label: string;
  color: string;
  count: number;
}

export interface KnowledgeEntry {
  id: string;
  accountId: string;
  acctColor: string;
  acctBadge: string;
  subject: string;
  excerpt: string;
  body: string;
  tags: string[];
  dateSent: number;
  usedCount: number;
  impact: string;
  lastUsedFor: string | null;
  lastUsedTime: number | null;
  source: string;
  thread: string;
  indexedAt: number | null;
}
