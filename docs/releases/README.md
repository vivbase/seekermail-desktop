# Release evidence (`docs/releases/`)

Per-release evidence archived by the Release Engineer at gate time (T105 / T106 /
T107). The gate scripts in `scripts/` produce or check these; the binary/visual
artifacts are captured against the built app and are **not** generated in CI.

| File | Produced by | Gate |
|---|---|---|
| `v0.5.0-beta_noproxy_check.png` | `docs/compliance/noproxy_check_sop.md` mitmproxy capture | T105 / T103 |
| `v0.6.0-beta_mis_send_drill.png` | Mis-send drill screenshot (3 mails all held as drafts) | T106 |
| `v0.7.0-rc_safety_report.json` | `cargo xtask safety-run --out …` archived copy | T107 / T104 |
| `v0.7.0-rc_noproxy_check.png` | mitmproxy capture (RC run) | T107 / T103 |
| `v0.7.0-rc_e5_blind_test.md` | E5 style-learning blind-test record (template committed) | T107 |
| `v0.7.0-rc_e7_csv_sample.csv` | E7 audit-log CSV export (redacted), `grep`-checked for denied fields | T107 |

Screenshots, the safety-report copy, and the CSV export are added by the Release
Engineer during the gate run; only the `*_e5_blind_test.md` record has a
committed template (below) to fill in.
