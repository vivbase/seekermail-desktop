// Shared back affordance for secondary pages (07 §4). SeekerMail uses fixed-parent
// navigation: every secondary surface (Pending, Unread, Processed, Report, GTE,
// Profile, Compose, Account Mail, Settings) declares the single parent it returns
// to, mirroring the prototype's `.pg-back` button. Primary rail pages (Dashboard,
// Inbox, Search, Team, Agents, Repository) are reachable from the sidebar and
// intentionally do NOT render this.
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

export type PageBackProps = {
  /** Fixed parent route this page returns to (e.g. "/", "/agents"). */
  to: string;
  /** Translation key in the `nav` namespace, e.g. "back_to_dashboard". */
  labelKey: string;
  /** Optional extra class names for per-page layout tuning. */
  className?: string;
};

/**
 * Chevron + "Back to {parent}" label. Navigates to a fixed parent route — never
 * browser history — so the destination is predictable from any entry point.
 */
export default function PageBack({ to, labelKey, className }: PageBackProps) {
  const navigate = useNavigate();
  const { t } = useTranslation("nav");
  return (
    <button
      type="button"
      className={className ? `pg-back ${className}` : "pg-back"}
      onClick={() => navigate(to)}
    >
      <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
        <path
          d="M9 2L4 7l5 5"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
      {t(labelKey)}
    </button>
  );
}
