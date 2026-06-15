# No-Proxy Egress Check — Release SOP (T103)

Manual network-capture procedure a Release Engineer runs once per release to
confirm the BYO-AI **no-proxy** architecture (ADR-0004): AI inference traffic
goes from the device straight to the user-configured provider, and **never**
through a SeekerMail-controlled server.

This complements the automated checks in `src-tauri/tests/compliance/`
(`cargo test --test compliance`), which assert the host invariant and log-safety
offline on every push. This SOP is the human, real-traffic confirmation.

## When

Before tagging any release that ships or changes AI features
(v0.5.0-beta onward). Archive the evidence under `docs/releases/`.

## Tools

- **mitmproxy** (`brew install mitmproxy`), or macOS `nettop` for a connection
  list, or Little Snitch's network monitor.
- A test cloud provider key (a throwaway key is fine; the check needs one real
  outbound inference call).

## Steps

1. Start the capture:
   - `mitmproxy` in a terminal, then run SeekerMail with the system HTTP(S)
     proxy pointed at `127.0.0.1:8080`; **or**
   - `sudo nettop -P -L 0 -t external` filtered to the SeekerMail process.
2. In SeekerMail: **Settings → AI Providers → Add Cloud Provider**, enter the
   test key, and run **Test Connection**.
3. Open any email → **AI Reply** (manual / E1 mode) to trigger one real
   inference request.
4. Let the capture run for ~60 seconds while you exercise one more AI action.
5. Stop the capture and review every outbound host.

## Pass criteria

- [ ] **No** request to `seekermail.app`, `api.seekermail.app`, `seekermail.com`,
      or any SeekerMail-controlled domain during AI activity.
- [ ] AI inference requests go **only** to the configured provider host
      (`api.openai.com` / `api.anthropic.com` / the custom base URL / `localhost`
      for a local model).
- [ ] The only SeekerMail-domain traffic at all is the update check
      (`updates.seekermail.app`), and never carries mail content.

## Evidence

Save a screenshot of the capture showing the provider host (and the absence of
SeekerMail inference traffic) to:

```
docs/releases/<version>_noproxy_check.png
```

e.g. `docs/releases/v0.5.0-beta_noproxy_check.png`. Reference it in the release
gate checklist (T105 / T106 / T107).

## If it fails

A request routing AI inference through a SeekerMail domain is a **release
blocker** and an ADR-0004 violation. File a P0, do not tag, and trace the
offending HTTP client back to its construction site.
