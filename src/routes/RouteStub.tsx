// Token-styled skeleton surface for a route (T007). v0.1 ships the shell + routing
// only; each page's real content lands with its feature card (v0.2+). This is a
// production-appropriate empty state — title, section label, and description — not
// a placeholder label.
import { useTranslation } from "react-i18next";

interface RouteStubProps {
  titleKey: string;
  descKey: string;
}

export default function RouteStub({ titleKey, descKey }: RouteStubProps) {
  const { t } = useTranslation(["nav", "common"]);
  return (
    <section className="mx-auto w-full max-w-3xl px-8 py-10">
      <p className="section-label mb-2">{t("nav:nav_section_overview")}</p>
      <h1 className="font-display text-4xl italic text-p10">{t(`nav:${titleKey}`)}</h1>
      <p className="mt-3 font-body text-p8">{t(`common:${descKey}`)}</p>
      <div className="mt-8 rounded-card border border-divider bg-surface p-6 shadow-card">
        <p className="font-body text-p7">{t("common:state_loading")}</p>
      </div>
    </section>
  );
}
