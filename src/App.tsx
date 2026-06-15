// Router (07 §4). The shell is a layout route; every page is a lazy child rendered
// into its <Outlet/> (route-level code-splitting, 07 §10). Onboarding sits outside
// the shell as the empty-account redirect target; the shell itself is gated on
// "≥1 account exists" (T046).
import { lazy } from "react";
import { createBrowserRouter, Navigate, RouterProvider } from "react-router-dom";

import AppShell from "@/components/layout/AppShell";
import { useHasAccounts } from "@/lib/accountGate";

const Onboarding = lazy(() => import("@/routes/onboarding"));
const Dashboard = lazy(() => import("@/routes/dashboard"));
const Pending = lazy(() => import("@/routes/pending"));
const Unread = lazy(() => import("@/routes/unread"));
const AllMail = lazy(() => import("@/routes/all-mail"));
const Processed = lazy(() => import("@/routes/processed"));
const Compose = lazy(() => import("@/routes/compose"));
const Gte = lazy(() => import("@/routes/gte"));
const Team = lazy(() => import("@/routes/team"));
const Repository = lazy(() => import("@/routes/repository"));
const Report = lazy(() => import("@/routes/report"));
const Agents = lazy(() => import("@/routes/agents"));
const Profile = lazy(() => import("@/routes/profile"));
const AccountEmails = lazy(() => import("@/routes/account-emails"));
const Search = lazy(() => import("@/routes/search"));
const MailDetail = lazy(() => import("@/routes/mail-detail"));
const SettingsShell = lazy(() => import("@/routes/settings/SettingsShell"));
const AccountsSettings = lazy(() => import("@/routes/settings/accounts"));
const AppearanceSettings = lazy(() => import("@/routes/settings/appearance"));
const PrivacySettings = lazy(() => import("@/routes/settings/privacy"));
const DataSettings = lazy(() => import("@/routes/settings/data"));
const DataExport = lazy(() => import("@/routes/settings/data/export"));
const DataWipe = lazy(() => import("@/routes/settings/data/wipe"));
const DataReindex = lazy(() => import("@/routes/settings/data/reindex"));
const DataSyncRange = lazy(() => import("@/routes/settings/data/sync_range"));
const DataFlowPanel = lazy(() => import("@/routes/settings/data/data_flow"));
const AiSettings = lazy(() => import("@/routes/settings/ai"));
const AiRecommendedSetup = lazy(() => import("@/routes/settings/ai/recommended"));
const AiProviderMatrix = lazy(() => import("@/routes/settings/ai/matrix"));
const AboutSettings = lazy(() => import("@/routes/settings/about"));

/** Routing gate (07 §4): the shell requires at least one account, else onboarding. */
function ShellGate() {
  const hasAccounts = useHasAccounts();
  return hasAccounts ? <AppShell /> : <Navigate to="/onboarding" replace />;
}

const router = createBrowserRouter([
  { path: "/onboarding", element: <Onboarding /> },
  {
    element: <ShellGate />,
    children: [
      { index: true, element: <Dashboard /> },
      { path: "pending", element: <Pending /> },
      { path: "unread", element: <Unread /> },
      { path: "all-mail", element: <AllMail /> },
      { path: "search", element: <Search /> },
      { path: "processed", element: <Processed /> },
      { path: "compose", element: <Compose /> },
      { path: "gte", element: <Gte /> },
      { path: "team", element: <Team /> },
      { path: "repository", element: <Repository /> },
      { path: "report", element: <Report /> },
      { path: "agents", element: <Agents /> },
      { path: "profile", element: <Profile /> },
      { path: "accounts/:id/mail", element: <AccountEmails /> },
      { path: "mail/:id", element: <MailDetail /> },
      // Settings shell (T049): a nested layout with its own left nav + Outlet.
      {
        path: "settings",
        element: <SettingsShell />,
        children: [
          { index: true, element: <Navigate to="/settings/accounts" replace /> },
          { path: "accounts", element: <AccountsSettings /> },
          { path: "appearance", element: <AppearanceSettings /> },
          { path: "privacy", element: <PrivacySettings /> },
          { path: "data", element: <DataSettings /> },
          { path: "data/export", element: <DataExport /> },
          { path: "data/wipe", element: <DataWipe /> },
          { path: "data/reindex", element: <DataReindex /> },
          { path: "data/sync-range", element: <DataSyncRange /> },
          { path: "data/data-flow", element: <DataFlowPanel /> },
          { path: "ai", element: <AiSettings /> },
          { path: "ai/recommended", element: <AiRecommendedSetup /> },
          { path: "ai/matrix", element: <AiProviderMatrix /> },
          { path: "about", element: <AboutSettings /> },
        ],
      },
    ],
  },
]);

export default function App() {
  return <RouterProvider router={router} />;
}
