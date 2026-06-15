// Settings → AI → Recommended Plan route (T064, F_F3 §5). The same wizard is
// also mountable as a modal from any AI-feature trigger when no provider is
// configured; this route is the always-available re-entry point the spec
// places under Settings → AI Providers → Recommended Plan.
import { useCallback } from "react";
import { useNavigate } from "react-router-dom";

import RecommendedSetupWizard from "./RecommendedSetupWizard";

export default function RecommendedSetupPage() {
  const navigate = useNavigate();
  const close = useCallback(() => navigate("/settings/ai"), [navigate]);
  return <RecommendedSetupWizard onClose={close} />;
}
