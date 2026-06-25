// Per-tab route table (WB-09 v2). The same page inventory as App.tsx, but laid out under
// WorkspaceLayout (Sidebar + Outlet, no global chrome) so each workbench tab can render this
// inside its OWN MemoryRouter for fully independent navigation. Mirrors App.tsx's children.
import { lazy } from "react";
import { Navigate, type RouteObject } from "react-router-dom";

import WorkspaceLayout from "@/components/layout/WorkspaceLayout";

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

export const workbenchRoutes: RouteObject[] = [
  {
    element: <WorkspaceLayout />,
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
];
