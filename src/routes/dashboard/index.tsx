// Dashboard (主界面) — 1:1 port of the prototype `page-dash` markup. Uses the
// ported prototype CSS classes (dash-body / stat-card / cold-card / lang picker)
// so it renders identically to UI/seekermail-unified.html. Numbers are live; the
// language picker switches the app locale.
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { applyLocale } from "@/i18n/applyLocale";
import { useAccounts } from "@/ipc/queries/accounts";
import { useMailCount } from "@/ipc/queries/mail";
import { usePendingQueriesCount } from "@/ipc/queries/queries";
import { useAiDecisionsSummary } from "@/ipc/queries/audit";

interface LangOpt {
  code: string;
  label: string;
  name: string;
  native: string;
  rtl?: boolean;
}
interface LangGroup {
  hdr: string;
  opts: LangOpt[];
}

// Region groups + native names mirror the prototype dropdown (native script text is
// the documented exception to the English-only UI rule).
const LANG_GROUPS: LangGroup[] = [
  {
    hdr: "Primary Markets",
    opts: [
      { code: "en", label: "EN", name: "English", native: "English" },
      { code: "zh-CN", label: "中", name: "Simplified Chinese", native: "简体中文" },
      { code: "zh-TW", label: "繁", name: "Traditional Chinese", native: "繁體中文" },
      { code: "ja", label: "日", name: "Japanese", native: "日本語" },
      { code: "ko", label: "한", name: "Korean", native: "한국어" },
    ],
  },
  {
    hdr: "Asia Pacific",
    opts: [
      { code: "vi", label: "VI", name: "Vietnamese", native: "Tiếng Việt" },
      { code: "hi", label: "हि", name: "Hindi", native: "हिन्दी" },
      { code: "bn", label: "বা", name: "Bengali", native: "বাংলা" },
    ],
  },
  {
    hdr: "Middle East & South Asia",
    opts: [
      { code: "ar", label: "ع", name: "Arabic", native: "العربية", rtl: true },
      { code: "he", label: "ע", name: "Hebrew", native: "עברית", rtl: true },
      { code: "ur", label: "اُ", name: "Urdu", native: "اردو", rtl: true },
    ],
  },
  {
    hdr: "Europe — Romance",
    opts: [
      { code: "es", label: "ES", name: "Spanish", native: "Español" },
      { code: "pt", label: "PT", name: "Portuguese", native: "Português" },
      { code: "fr", label: "FR", name: "French", native: "Français" },
      { code: "it", label: "IT", name: "Italian", native: "Italiano" },
    ],
  },
  {
    hdr: "Europe — Germanic & Other",
    opts: [
      { code: "de", label: "DE", name: "German", native: "Deutsch" },
      { code: "nl", label: "NL", name: "Dutch", native: "Nederlands" },
      { code: "sv", label: "SV", name: "Swedish", native: "Svenska" },
      { code: "pl", label: "PL", name: "Polish", native: "Polski" },
      { code: "ru", label: "RU", name: "Russian", native: "Русский" },
      { code: "tr", label: "TR", name: "Turkish", native: "Türkçe" },
    ],
  },
];

const ALL_LANGS = LANG_GROUPS.flatMap((g) => g.opts);

export default function Dashboard() {
  const { t, i18n } = useTranslation("dashboard");
  const navigate = useNavigate();
  const [coldOpen, setColdOpen] = useState(true);
  const [langOpen, setLangOpen] = useState(false);

  const accounts = useAccounts();
  const accountCount = accounts.data?.length ?? 0;
  const pending = usePendingQueriesCount();
  const total = useMailCount({});
  const unread = useMailCount({ isUnread: true });

  const since = useMemo(() => {
    const d = new Date();
    d.setHours(0, 0, 0, 0);
    return Math.floor(d.getTime() / 1000);
  }, []);
  const now = useMemo(() => Math.floor(Date.now() / 1000), []);
  const processed = useAiDecisionsSummary(null, since, now);

  const nf = useMemo(() => new Intl.NumberFormat(), []);
  const fmt = (n?: number) => (n == null ? "—" : nf.format(n));

  const current = ALL_LANGS.find((o) => o.code === i18n.language) ?? {
    code: "en",
    label: "EN",
    name: "English",
    native: "English",
  };
  const setLang = (code: string) => {
    applyLocale(code);
    void i18n.changeLanguage(code);
    setLangOpen(false);
  };

  return (
    <div className="dash-body" style={{ height: "100%" }}>
      {/* Page header: title + language switcher */}
      <div
        style={{
          display: "flex",
          alignItems: "flex-end",
          justifyContent: "space-between",
          flexShrink: 0,
          marginBottom: 4,
        }}
      >
        <div className="pg-title">{t("title")}</div>
        <div className="lang-picker-wrap">
          <button className="lang-btn" onClick={() => setLangOpen((o) => !o)} aria-label="Change language">
            <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
              <circle cx="6.5" cy="6.5" r="5.5" stroke="currentColor" strokeWidth="1.2" />
              <path
                d="M6.5 1c0 0-2.5 2-2.5 5.5s2.5 5.5 2.5 5.5M6.5 1c0 0 2.5 2 2.5 5.5S6.5 12 6.5 12M1 6.5h11"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
              />
            </svg>
            <span>{current.label}</span>
            <svg width="8" height="8" viewBox="0 0 8 8" fill="none">
              <path d="M1.5 2.5l2.5 3 2.5-3" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
            </svg>
          </button>
          <div className={`lang-dropdown${langOpen ? " open" : ""}`}>
            {LANG_GROUPS.map((g) => (
              <div key={g.hdr}>
                <div className="lang-group-hdr">{g.hdr}</div>
                {g.opts.map((o) => (
                  <div
                    key={o.code}
                    className={`lang-opt${o.code === i18n.language ? " active" : ""}`}
                    onClick={() => setLang(o.code)}
                  >
                    <span className="lang-opt-name">{o.name}</span>
                    <span className="lang-opt-native">{o.native}</span>
                    {o.rtl && <span className="lang-opt-rtl">RTL</span>}
                  </div>
                ))}
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Stat row */}
      <div className="stat-row">
        <div className="stat-card card-terra" onClick={() => navigate("/pending")}>
          <div className="lbl">{t("stat_pending_lbl")}</div>
          <div className="num">{fmt(pending.data)}</div>
          <div className="sub">{t("stat_pending_sub")}</div>
          <div className="tap-hint">{t("stat_view_all")}</div>
        </div>
        <div className="stat-card card-slate" onClick={() => navigate("/unread")}>
          <div className="lbl">{t("stat_unread_lbl")}</div>
          <div className="num">{fmt(unread.data)}</div>
          <div className="sub">{t("stat_unread_sub", { count: accountCount })}</div>
          <div className="tap-hint">{t("stat_view_all")}</div>
        </div>
        <div className="stat-card card-sage" onClick={() => navigate("/all-mail")}>
          <div className="lbl">{t("stat_total_lbl")}</div>
          <div className="num">{fmt(total.data)}</div>
          <div className="sub">{t("stat_total_sub", { count: accountCount })}</div>
          <div className="tap-hint">{t("stat_view_all")}</div>
        </div>
        <div className="stat-card card-amber" onClick={() => navigate("/processed")}>
          <div className="lbl">{t("stat_processed_lbl")}</div>
          <div className="num">{fmt(processed.data?.totalEvents)}</div>
          <div className="sub">{t("stat_processed_sub")}</div>
          <div className="tap-hint">{t("stat_view_log")}</div>
        </div>
      </div>

      {/* Cold-start checklist */}
      {coldOpen && (
        <div className="cold-card">
          <div className="cold-head">
            <div className="cold-title">{t("cold_title")}</div>
            <button className="cold-x" onClick={() => setColdOpen(false)} title={t("cold_dismiss")}>
              ×
            </button>
          </div>
          <div className="cold-note">{t("cold_note")}</div>
          <div className="cold-tasks">
            <div className="cold-task done">
              <span className="cold-check">✓</span>
              <span className="ct-txt">{t("cold_task_connect")}</span>
            </div>
            <div className="cold-task" onClick={() => navigate("/pending")}>
              <span className="cold-check"></span>
              <span className="ct-txt">{t("cold_task_drafts")}</span>
              <span className="ct-meta">0 / 5</span>
            </div>
            <div className="cold-task" onClick={() => navigate("/agents")}>
              <span className="cold-check"></span>
              <span className="ct-txt">{t("cold_task_contacts")}</span>
              <span className="ct-meta">0 / 3</span>
            </div>
            <div className="cold-task" onClick={() => navigate("/gte")}>
              <span className="cold-check"></span>
              <span className="ct-txt">{t("cold_task_search")}</span>
            </div>
          </div>
          <div className="cold-prog">{t("cold_progress")}</div>
        </div>
      )}

      {/* Daily report link */}
      <div className="report-bar">
        <span className="report-link" onClick={() => navigate("/report")}>
          {t("report_link")}
        </span>
      </div>
      <div className="dash-placeholder">{t("hint")}</div>
    </div>
  );
}
