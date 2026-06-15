// Mail body renderer (T041). Wraps SanitizedMail for HTML bodies, or falls back
// to a plain-text view. Shows an empty-state message when there is no content.
// SanitizedMail handles the DOMPurify second pass internally (T028) and can
// optionally render TrackerBadge + RemoteImageBar (T029) when trackerInfo is
// provided. We pass trackerInfo here so both the badge and image bar appear
// at the top of the body column — the natural reading-pane position.
import { useTranslation } from "react-i18next";
import type { MailDetail, TrackerInfo } from "@shared/bindings";

import { useSelection } from "@/stores/selection";
import SanitizedMail from "./SanitizedMail";

interface MailBodyProps {
  mail: MailDetail;
  /** When provided, renders TrackerBadge + RemoteImageBar inside SanitizedMail. */
  trackerInfo?: TrackerInfo;
}

export function MailBody({ mail, trackerInfo }: MailBodyProps) {
  const { t } = useTranslation("reading");
  // D1 legal excerpt highlight (T071 §3.3) — set by LegalSidebar risk clicks,
  // cleared automatically when the selected mail changes (selection store).
  const legalHighlightText = useSelection((s) => s.legalHighlightText);

  const hasContent = !!(mail.bodyHtml || mail.bodyText);

  if (!hasContent) {
    return (
      <div className="py-8 text-center">
        <p className="font-body text-sm italic text-p7">{t("no_body")}</p>
      </div>
    );
  }

  return (
    <div className="py-6">
      {/* SanitizedMail runs DOMPurify defence-in-depth pass internally (T028).
          TrackerBadge + RemoteImageBar render inside it when trackerInfo is set. */}
      <SanitizedMail
        bodyHtml={mail.bodyHtml}
        bodyText={mail.bodyText}
        mailId={mail.id}
        trackerInfo={trackerInfo}
        highlightPhrase={legalHighlightText}
      />
    </div>
  );
}
