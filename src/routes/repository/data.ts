// Repository seed data — ported 1:1 from the prototype (UI/seekermail-unified.html
// KNOWLEDGE / TODAY_DECISIONS / renderAcctBreakdown / renderTopicChart). Realistic
// English content; the live equivalents are served by the GTE search + audit
// backends once those command surfaces land. All copy is English (root CLAUDE.md).

export type AcctKey = "legal" | "work" | "person";
export type ImpactKind = "risk" | "reply" | "identity" | "rule" | "context";

export interface KnowledgeItem {
  id: number;
  acct: AcctKey;
  acctColor: string;
  acctLbl: string;
  acctTc?: string;
  title: string;
  excerpt: string;
  body: string;
  tags: string[];
  date: string;
  usedCount: number;
  impact: ImpactKind;
  lastUsedFor: string | null;
  lastUsedEmail: string | null;
  lastUsedTime: string | null;
  meta: { source: string; thread: string; indexed: string };
}

export const KNOWLEDGE: KnowledgeItem[] = [
  {
    id: 1,
    acct: "legal",
    acctColor: "var(--terra)",
    acctLbl: "L",
    title: "Q3 Service Contract — Non-Compete Clause Analysis",
    excerpt:
      "boss@corp.com requested adding a non-compete restriction in Clause 12, barring Party B from working in similar industries for two years post-contract — exceeds standard NDA scope.",
    body: "<p>Key findings:</p><p>The counterparty added a non-compete requirement in the Q3 renewal: 24-month restriction post-expiry covering “same or similar industries.” Three issues:</p><p>① “Similar industry” is undefined and overly broad; ② conflicts with our internal employee compliance policy; ③ exceeds the reasonable scope of a service contract.</p><p>Recommendation: reject this amendment; retain original Clause 12 standard confidentiality terms. If counterparty insists, propose narrowing scope to specific business lines.</p>",
    tags: ["Contract", "NDA", "Compliance"],
    date: "2026-04-23",
    usedCount: 4,
    impact: "rule",
    lastUsedFor:
      "AI identified non-compete clause as exceeding standard scope; drafted negotiation reply recommending rejection",
    lastUsedEmail: "Q3 Contract Renewal",
    lastUsedTime: "Today 09:31",
    meta: { source: "boss@corp.com", thread: "Q3 Service Contract Renewal", indexed: "Today 09:14" },
  },
  {
    id: 2,
    acct: "work",
    acctColor: "var(--slate)",
    acctLbl: "W",
    title: "Vendor Inc. Payment History",
    excerpt:
      "14 payments totalling ¥612,000 over 18 months, all from @vendor.com with no account changes. Anomaly flagged: @vendor.io domain appeared on 2026-04-23.",
    body: "<p>Vendor Inc. payment summary:</p><p>• Period: 18 months (Oct 2024 – Apr 2026)<br>• Total: 14 payments · ¥612,000<br>• Historical domain: @vendor.com (all records)<br>• Beneficiary: ICBC 622x-xxxx-xxxx-4412 (unchanged)</p><p>⚠ Anomaly: ¥48,000 request from @vendor.io on 2026-04-23 — domain mismatch vs history. T4 risk alert triggered, manual review pending.</p>",
    tags: ["Payment", "Vendor"],
    date: "2026-04-23",
    usedCount: 3,
    impact: "risk",
    lastUsedFor:
      "Detected @vendor.io ≠ @vendor.com — triggered T4 risk alert and paused automatic processing",
    lastUsedEmail: "Payment Request #4471",
    lastUsedTime: "Today 10:22",
    meta: { source: "ap@vendor.com", thread: "Payment Confirmation Thread", indexed: "Today 10:22" },
  },
  {
    id: 3,
    acct: "legal",
    acctColor: "var(--terra)",
    acctLbl: "L",
    title: "Standard NDA Template v2.3",
    excerpt:
      "Company NDA template for third-party collaboration, vendor onboarding, and project outsourcing. Default validity 24 months; includes non-compete exemption clause.",
    body: "<p>NDA template key terms:</p><p>• Validity: 24 months (extendable up to 36 months)<br>• Scope: all non-public information shared during the engagement<br>• Non-compete: not included by default (separate agreement required)<br>• Disputes: China International Economic and Trade Arbitration Commission</p><p>Recent use: Q3 contract renewal — counterparty requested non-compete addition, rejected.</p>",
    tags: ["NDA", "Contract"],
    date: "2026-03-15",
    usedCount: 6,
    impact: "rule",
    lastUsedFor:
      "Compared counterparty clause against standard NDA — confirmed clause exceeds scope, supporting AI rejection recommendation",
    lastUsedEmail: "Q3 Contract Renewal",
    lastUsedTime: "Today 09:31",
    meta: { source: "Internal document", thread: "Legal Template Library", indexed: "2026-03-15" },
  },
  {
    id: 4,
    acct: "work",
    acctColor: "var(--slate)",
    acctLbl: "W",
    title: "Q3 Vendor Quote Comparison",
    excerpt:
      "Three quotes received: Vendor Inc. ¥48,000, SupplyCo ¥52,000, FastParts ¥44,500. FastParts lowest unit price but delivery 2 weeks late.",
    body: "<p>Quote comparison (Q3 Technical Services):</p><p>Vendor A — Vendor Inc.: ¥48,000, 10-day lead, 18-month history<br>Vendor B — SupplyCo: ¥52,000, 7-day lead, first engagement<br>Vendor C — FastParts: ¥44,500, 24-day lead, 6-month history</p><p>Assessment: Vendor Inc. balances price and lead time with a clean payment record. Recommended — pending domain anomaly investigation.</p>",
    tags: ["Quote", "Vendor"],
    date: "2026-04-20",
    usedCount: 2,
    impact: "context",
    lastUsedFor:
      "Provided Vendor Inc. historical quote context to validate ¥48,000 amount in payment request evaluation",
    lastUsedEmail: "Payment Request #4471",
    lastUsedTime: "Today 10:22",
    meta: { source: "ap@vendor.com", thread: "Q3 Procurement RFQ", indexed: "2026-04-20" },
  },
  {
    id: 5,
    acct: "legal",
    acctColor: "var(--terra)",
    acctLbl: "L",
    title: "Annual Compliance Checklist 2026",
    excerpt:
      "Regulatory review items: data privacy, contract archiving, vendor credential audit, and anti-corruption policy updates — all due by 2026-06-30.",
    body: "<p>2026 annual compliance items:</p><p>① Data privacy: complete Personal Information Protection Impact Assessment (due 2026-05-31)<br>② Contract archiving: clear pre-2024 expired contracts and complete digital archiving (due 2026-06-30)<br>③ Vendor credentials: re-audit all active vendor licences and certifications (due 2026-06-15)<br>④ Anti-corruption training: all staff complete annual integrity compliance training (due 2026-06-30)</p>",
    tags: ["Compliance", "Contract"],
    date: "2026-04-01",
    usedCount: 1,
    impact: "context",
    lastUsedFor:
      "AI generated a structured compliance progress reply when processing annual compliance reminder email",
    lastUsedEmail: "Annual Compliance Reminder",
    lastUsedTime: "Yesterday 14:20",
    meta: { source: "compliance@internal", thread: "Annual Compliance Plan", indexed: "2026-04-01" },
  },
  {
    id: 6,
    acct: "work",
    acctColor: "var(--slate)",
    acctLbl: "W",
    title: "Q3 Delivery Schedule Confirmed",
    excerpt:
      "PM confirmed Q3 milestones: feature freeze Jun 20, UAT Jul 1, launch Jul 15. Three high-risk delay points flagged.",
    body: "<p>Q3 delivery timeline (confirmed):</p><p>• Requirements freeze: 2026-06-01<br>• Feature complete: 2026-06-20<br>• UAT start: 2026-07-01<br>• Target launch: 2026-07-15<br>• Acceptance deadline: 2026-07-31</p><p>⚠ Risks: ① vendor API docs delayed; ② legal review ~5 business days; ③ public holiday impact (Jul 4).</p>",
    tags: ["Schedule", "Project"],
    date: "2026-04-18",
    usedCount: 2,
    impact: "reply",
    lastUsedFor:
      "AI extracted schedule data to generate an accurate status update draft when replying to project progress enquiry",
    lastUsedEmail: "Project Status Sync",
    lastUsedTime: "Yesterday 11:05",
    meta: { source: "pm@co.com", thread: "Q3 Project Schedule", indexed: "2026-04-18" },
  },
  {
    id: 7,
    acct: "work",
    acctColor: "var(--slate)",
    acctLbl: "W",
    title: "Vendor Payment Risk — Domain Anomaly",
    excerpt:
      "@vendor.io detected in a payment request on 2026-04-23; all history shows @vendor.com. GTE flagged T4 high-risk — manual review triggered.",
    body: "<p>Risk event details:</p><p>• Sender: ap@vendor.io (anomalous)<br>• Historical sender: ap@vendor.com (normal)<br>• Amount: ¥48,000<br>• Detected: 2026-04-23 10:18<br>• Type: T4 — domain mismatch (suspected phishing)</p><p>Action: decline authorization until phone confirmation with vendor finance. Do not approve based on email alone.</p>",
    tags: ["Payment", "Compliance", "Vendor"],
    date: "2026-04-23",
    usedCount: 3,
    impact: "risk",
    lastUsedFor:
      "Provided evidence basis for AI T4 alert card — required manual confirmation before processing",
    lastUsedEmail: "Payment Request #4471",
    lastUsedTime: "Today 10:23",
    meta: { source: "ap@vendor.io", thread: "Payment Request Risk Record", indexed: "Today 10:23" },
  },
  {
    id: 8,
    acct: "person",
    acctColor: "var(--sage)",
    acctLbl: "P",
    acctTc: "var(--p10)",
    title: "Q1 2026 Meeting Minutes Summary",
    excerpt:
      "Consolidated 7 Q1 project meeting minutes covering product roadmap, vendor review, and compliance training. Key decisions extracted and archived.",
    body: "<p>Q1 meeting summary (Jan – Mar 2026):</p><p>7 sessions · key decisions:<br>① v0.7 Public Beta roadmap confirmed (Jan 15)<br>② Vendor Inc. selected as primary Q2–Q3 supplier (Feb 10)<br>③ Annual compliance review plan approved (Mar 5)<br>④ Agent-IM module added to v0.8 development plan (Mar 20)</p>",
    tags: ["Schedule", "Project"],
    date: "2026-04-05",
    usedCount: 1,
    impact: "context",
    lastUsedFor: "Provided background on Q2 vendor selection decision to support new procurement RFQ",
    lastUsedEmail: "Q2 Procurement Planning",
    lastUsedTime: "3 days ago",
    meta: { source: "personal@me.com", thread: "Meeting Minutes Archive", indexed: "2026-04-05" },
  },
  {
    id: 9,
    acct: "legal",
    acctColor: "var(--terra)",
    acctLbl: "L",
    title: "Service Contract Template Library v4.1",
    excerpt:
      "Six standard templates: technical services, consulting, NDA, procurement, joint development, and agency — all cleared by legal review.",
    body: "<p>Current template inventory:</p><p>1. Technical Services Contract v4.1 (updated 2026-03-01)<br>2. Consulting Services Contract v3.2<br>3. Non-Disclosure Agreement v2.3<br>4. Procurement Contract v2.8<br>5. Joint Development Agreement v1.4<br>6. Agency Contract v2.0</p><p>All templates have passed internal legal review. Scope and usage notes on cover page of each file.</p>",
    tags: ["Contract", "NDA"],
    date: "2026-03-01",
    usedCount: 5,
    impact: "rule",
    lastUsedFor:
      "AI called this template to extract standard clause recommendations when drafting Q3 renewal negotiation reply",
    lastUsedEmail: "Q3 Contract Renewal",
    lastUsedTime: "Today 09:31",
    meta: { source: "legal@co.com", thread: "Legal Template Library", indexed: "2026-03-01" },
  },
  {
    id: 10,
    acct: "person",
    acctColor: "var(--sage)",
    acctLbl: "P",
    acctTc: "var(--p10)",
    title: "Contact Profile — boss@corp.com",
    excerpt:
      "23 historical emails on contract renewals and business collaboration. Decision style favours written confirmation; replies typically weekday mornings 9–11 AM.",
    body: "<p>Contact profile (from email history):</p><p>• Email: boss@corp.com<br>• Organisation: corp.com<br>• Frequency: 2–4 emails/month<br>• Topics: contract terms, quarterly reviews<br>• Style: formal, requires clear action items<br>• Reply window: weekdays 09:00–11:00, no holiday replies</p><p>Latest interaction: 2026-04-23 — Q3 non-compete clause discussion.</p>",
    tags: ["Vendor"],
    date: "2026-04-23",
    usedCount: 7,
    impact: "identity",
    lastUsedFor:
      "AI recognised familiar contact; generated reply matching their formal style with explicit action items",
    lastUsedEmail: "Q3 Contract Renewal",
    lastUsedTime: "Today 09:31",
    meta: { source: "boss@corp.com", thread: "Contact Profile", indexed: "Today 09:15" },
  },
  {
    id: 11,
    acct: "work",
    acctColor: "var(--slate)",
    acctLbl: "W",
    title: "Vendor Credential Archive",
    excerpt:
      "Business licences, certifications, and bank details for 3 active vendors archived. Next audit: 2026-06-15. SupplyCo licence expiry flagged.",
    body: "<p>Active vendor credentials:</p><p>Vendor Inc.: business licence (valid to Dec 2027), VAT taxpayer cert, bank account permit<br>SupplyCo: business licence (expires Nov 2026 — renewal follow-up needed), ISO quality cert<br>FastParts: business licence (valid to Mar 2028), ISO 9001</p><p>⚠ SupplyCo licence expires Nov 2026. Follow up on renewal by Sep 2026.</p>",
    tags: ["Vendor", "Compliance"],
    date: "2026-04-10",
    usedCount: 2,
    impact: "context",
    lastUsedFor:
      "Found no registered domain change in Vendor Inc. credentials — supported classifying the payment request as high-risk",
    lastUsedEmail: "Payment Request #4471",
    lastUsedTime: "Today 10:22",
    meta: { source: "procurement@co.com", thread: "Vendor Records Management", indexed: "2026-04-10" },
  },
  {
    id: 12,
    acct: "person",
    acctColor: "var(--sage)",
    acctLbl: "P",
    acctTc: "var(--p10)",
    title: "Newsletter Digest — Q1 2026",
    excerpt:
      "84 subscription emails in Q1 (tech newsletters 42, industry news 31, event invites 11), archived by topic. Three items flagged as high-value references.",
    body: "<p>Q1 subscription digest:</p><p>Tech (42): AI tooling updates, developer newsletters<br>Industry (31): legal tech, SaaS market reports<br>Events (11): conference invitations, webinars</p><p>High-value flags:<br>① Anthropic model update notes (Feb 2026)<br>② Local LLM deployment best practices (Mar 2026)<br>③ Enterprise AI compliance guide (Mar 2026)</p>",
    tags: ["Schedule"],
    date: "2026-04-02",
    usedCount: 0,
    impact: "context",
    lastUsedFor: null,
    lastUsedEmail: null,
    lastUsedTime: null,
    meta: { source: "personal@me.com", thread: "Subscription Archive", indexed: "2026-04-02" },
  },
];

export interface TodayDecision {
  time: string;
  email: string;
  action: string;
  impact: ImpactKind;
  basis: string;
  result: string;
}

export const TODAY_DECISIONS: TodayDecision[] = [
  {
    time: "10:22",
    email: "Payment Request #4471 · AP Finance",
    action: "AI detected payment risk — paused processing",
    impact: "risk",
    basis:
      "[Vendor Inc. payment history] All 18-month transactions originated from @vendor.com; this request came from @vendor.io — domain mismatch detected",
    result: "T4 risk alert triggered · Inquiry card sent, awaiting your confirmation",
  },
  {
    time: "09:31",
    email: "Q3 Contract Renewal · Boss Senior",
    action: "AI generated negotiation reply draft",
    impact: "reply",
    basis:
      "[Contact profile: boss@corp.com] Prefers formal written comms, requires explicit action items; [Standard NDA Template v2.3] Non-compete clause not included by default",
    result: "Draft ready · Recommends rejecting non-compete clause, retaining standard NDA terms",
  },
  {
    time: "08:55",
    email: "Job Enquiry · Recruiter Jane",
    action: "AI auto-replied",
    impact: "reply",
    basis:
      "[Reply style history] Past responses to recruiter enquiries: polite decline, concise and non-committal tone",
    result: "Auto-reply sent · Politely declined the opportunity",
  },
  {
    time: "Yesterday 14:20",
    email: "Annual Compliance Review · Compliance Team",
    action: "AI generated compliance progress reply",
    impact: "rule",
    basis:
      "[2026 Annual Compliance Checklist] Extracted deadlines and current status per item, generated structured progress summary",
    result: "Draft pushed to review queue · Awaiting your review and send approval",
  },
];

export const IMPACT_ICON: Record<ImpactKind, string> = {
  risk: "⚠",
  reply: "↩",
  identity: "👤",
  rule: "📋",
  context: "📎",
};

export interface AcctBreakdownRow {
  lbl: string;
  addr: string;
  color: string;
  n: number;
  max: number;
  tc?: string;
}

export const ACCT_BREAKDOWN: AcctBreakdownRow[] = [
  { lbl: "Legal Account", addr: "legal@co.com", color: "var(--terra)", n: 27800, max: 48200 },
  { lbl: "Work Account", addr: "work@co.com", color: "var(--slate)", n: 48200, max: 48200 },
  { lbl: "Personal", addr: "personal@me.com", color: "var(--sage)", n: 15450, max: 48200, tc: "var(--p10)" },
];

export interface TopicChartRow {
  lbl: string;
  n: number;
  color: string;
}

export const TOPIC_CHART: TopicChartRow[] = [
  { lbl: "Vendor", n: 18, color: "var(--slate)" },
  { lbl: "Payment", n: 14, color: "var(--amber)" },
  { lbl: "Contract", n: 11, color: "var(--terra)" },
  { lbl: "NDA", n: 9, color: "var(--slate-d, #8A96A8)" },
  { lbl: "Compliance", n: 6, color: "var(--green)" },
  { lbl: "Schedule", n: 4, color: "var(--p8)" },
];

export interface SuggestItem {
  icon: string;
  text: string;
  sub?: string;
  query: string;
}

export const SUGGEST_PEOPLE: SuggestItem[] = [
  { icon: "👤", text: "boss@corp.com", sub: "23 emails", query: "boss@corp.com" },
  { icon: "👤", text: "AP Finance · ap@vendor.com", sub: "14 payments", query: "AP Finance vendor" },
  { icon: "👤", text: "Recruiter Jane", sub: "talentco.com", query: "Recruiter Jane" },
];

export const SUGGEST_TOPICS: SuggestItem[] = [
  { icon: "💳", text: "Vendor payment & domain anomaly", query: "vendor payment domain" },
  { icon: "📄", text: "NDA · non-compete clause", query: "NDA non-compete contract" },
  { icon: "✅", text: "Compliance deadlines 2026", query: "compliance deadline" },
  { icon: "📅", text: "Q3 project milestones", query: "Q3 milestones schedule" },
];

export const SUGGEST_TRY: SuggestItem[] = [
  { icon: "🔍", text: "Any payment risk or anomaly?", query: "payment risk anomaly" },
  { icon: "🔍", text: "What contract renewals are pending?", query: "contract renewal terms" },
  { icon: "🔍", text: "Vendor qualification documents", query: "vendor qualification" },
];

export interface QuickChip {
  label: string;
  query: string;
  title?: string;
}

export const QUICK_CHIPS: QuickChip[] = [
  { label: "vendor domain mismatch", query: "vendor payment domain mismatch" },
  { label: "boss@corp.com", query: "boss@corp.com contract" },
  { label: "NDA non-compete", query: "NDA non-compete", title: "Non-Disclosure Agreement" },
  { label: "compliance deadline", query: "compliance deadline 2026" },
  { label: "¥48,000 payment", query: "¥48000 payment" },
];
