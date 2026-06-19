// SeekerMail ID — client hooks for the OPTIONAL, mailbox-independent identity (A6).
//
// The SeekerMail ID is created by signing in with Google (OIDC) and is INDEPENDENT
// of imported mailboxes: it carries login, entitlement, and the OPT-IN marketing
// contact email. It is OPTIONAL — the app is fully usable locally with no identity.
// Signing out clears only the identity; mailboxes and local mail are untouched
// (this replaces the old "binding mailbox" model where sign-out removed every
// mailbox). Google sign-in itself is stubbed in the backend until the cloud-identity
// service ships (T121); the local identity row, sign-out, and marketing-consent
// plumbing are fully functional now.
//
// Spec: knowledge base `docs/function list/F_A6_seekermail_id.md` (rewritten) and
// `docs/analysis/26_identity_decoupling_and_email_marketing_foundation.md`.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { SeekerMailId } from "@shared/bindings";

import { ipc } from "../client";

export const identityKeys = {
  all: ["seekermail_id"] as const,
};

/** The current SeekerMail ID, or `null` when signed out (the local-first default). */
export function useSeekerMailId() {
  return useQuery({
    queryKey: identityKeys.all,
    queryFn: () => ipc("get_seekermail_id"),
  });
}

/**
 * Sign out of the SeekerMail ID. Clears only the identity — mailboxes and local
 * mail are untouched (identity is independent of data sources). The authoritative
 * server-side session revoke arrives with the cloud-identity backend.
 */
export function useSignOutSeekerMail() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => ipc("sign_out_seekermail"),
    onSuccess: () => void qc.invalidateQueries({ queryKey: identityKeys.all }),
  });
}

/** Set or withdraw the marketing-consent flag (opt-in; default OFF, first-party only). */
export function useSetMarketingConsent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { consent: boolean; source?: string }) =>
      ipc("set_marketing_consent", { consent: vars.consent, source: vars.source ?? null }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: identityKeys.all }),
  });
}

/**
 * Sign in with Google for the SeekerMail ID (OIDC; scopes `openid email profile`
 * — NO mail access, distinct from connecting a Gmail mailbox). Loopback flow
 * (analysis/27): `begin_google_signin` opens the system browser and starts a local
 * listener; `complete_google_signin` then waits for the redirect, verifies the
 * id_token, and returns the identity. The backend captures the code via loopback,
 * so the client passes an empty `code` plus the `state` returned by begin.
 */
export function useGoogleSignIn() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const begun = await ipc("begin_google_signin");
      return ipc("complete_google_signin", { code: "", state_nonce: begun.state });
    },
    onSuccess: () => void qc.invalidateQueries({ queryKey: identityKeys.all }),
  });
}

export type { SeekerMailId };
