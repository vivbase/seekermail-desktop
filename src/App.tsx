// Router (07 §4). The app is a tab-driven workbench: WorkbenchShell renders the global
// chrome plus one fully independent MemoryRouter per tab. The outer data router here exists
// only to (a) host the first-run onboarding route and (b) give the global chrome a navigation
// context (e.g. the risk banner). The shell drives its content from the tab store, NOT this
// URL, so it needs no per-page routes here — a catch-all maps every other path to the shell.
// That catch-all also matches the "index.html" a detached "Open in new window" loads, so a
// new window boots to its tab instead of rendering a 404.
import { lazy } from "react";
import { createBrowserRouter, RouterProvider } from "react-router-dom";

import WorkbenchShell from "@/components/workbench/WorkbenchShell";

const Onboarding = lazy(() => import("@/routes/onboarding"));

/** Routing gate (07 §4): the shell requires at least one account, else onboarding.
 *  WorkbenchShell runs that "≥1 account" gate (→ /onboarding) internally. */
function ShellGate() {
  return <WorkbenchShell />;
}

const router = createBrowserRouter([
  { path: "/onboarding", element: <Onboarding /> },
  // Catch-all → the workbench shell. Includes "/" (main window) and "/index.html?boot=…"
  // (a detached window), so every entry point resolves to the shell rather than 404-ing.
  { path: "*", element: <ShellGate /> },
]);

export default function App() {
  return <RouterProvider router={router} />;
}
