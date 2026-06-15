// About settings page (T049). Read-only: product name, version, tagline.
// Version is read from import.meta.env.VITE_APP_VERSION (injected by Vite from
// tauri.conf.json at build time). Falls back to "0.1.0" in dev/test.
import { useTranslation } from "react-i18next";

import BrandMark from "@/components/brand/BrandMark";

const APP_VERSION: string = (import.meta.env.VITE_APP_VERSION as string | undefined) ?? "0.1.0";

export default function AboutSettings() {
  const { t } = useTranslation("settings");

  return (
    <div className="max-w-xl space-y-6">
      {/* Brand block */}
      <div className="flex flex-col gap-2">
        <BrandMark size={48} />
        <h2 className="font-display text-3xl italic text-p10">SeekerMail</h2>
        <p className="font-body text-sm text-p8">{t("about_tagline")}</p>
      </div>

      {/* Meta table */}
      <dl className="divide-y divide-divider rounded-card border border-divider bg-surface px-4">
        <MetaRow term={t("about_version")} value={APP_VERSION} mono />
        <MetaRow term="Platform" value="Tauri + Rust + React 18" />
        <MetaRow term="Storage" value="Local (LanceDB + SQLite)" />
      </dl>

      {/* Statements */}
      <div className="space-y-2 rounded-card border border-divider bg-surface px-4 py-4">
        <p className="font-body text-sm text-p9">{t("about_built_with")}</p>
        <p className="font-body text-sm text-p9">{t("about_local_first")}</p>
      </div>

      <p className="font-body text-xs text-p8">{t("about_copyright")}</p>
    </div>
  );
}

// ── Meta row ──────────────────────────────────────────────────────────────────

function MetaRow({ term, value, mono = false }: { term: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between py-2.5">
      <dt className="font-ui text-xs uppercase tracking-wider text-p8">{term}</dt>
      <dd className={mono ? "font-mono text-sm text-p9" : "font-body text-sm text-p9"}>{value}</dd>
    </div>
  );
}
