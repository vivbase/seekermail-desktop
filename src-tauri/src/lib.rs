//! SeekerMail backend library root — the module tree and the Tauri app builder.
//!
//! `main.rs` is a one-line shim that calls [`run`]. Keeping the builder in the
//! library (the Tauri 2 idiom) lets integration tests and a future mobile target
//! share it.

pub mod account;
pub mod ai;
pub mod commands;
pub mod config;
pub mod embedding;
pub mod error;
pub mod events;
pub mod exporter;
pub mod extraction;
pub mod identity;
pub mod imap;
pub mod keychain;
pub mod logging;
pub mod net;
pub mod recovery;
pub mod sanitize;
pub mod search;
pub mod send;
pub mod state;
pub mod storage;
pub mod types;
pub mod util;
pub mod vector;

/// specta → TypeScript bindings exporter. Compiled only for `pnpm gen:types`
/// (the `gen-bindings` bin), never into the shipping app.
#[cfg(feature = "specta-export")]
pub mod export;

pub use state::AppState;

use config::Paths;

/// Build and run the Tauri application.
///
/// Order: resolve paths → init logging → in `setup`, bootstrap state, start the
/// parse worker + sync scheduler, manage both, and register every command via the
/// native `generate_handler!` (T002). New commands are appended to that list.
pub fn run() {
    // Load .env if present (dev convenience; silently ignored in production
    // when no .env exists). Must run before any std::env::var call.
    dotenvy::dotenv().ok();

    let paths = Paths::resolve().expect("resolve application paths");
    paths.ensure_dirs().expect("create application directories");

    // The guard flushes the non-blocking log writer; keep it alive until `run`
    // returns (process exit).
    let _log_guard = logging::init(&paths).expect("initialize logging");
    tracing::info!(event = "startup", "SeekerMail starting");

    tauri::Builder::default()
        // OS notifications for batched E2 drafts (T083). Registered before
        // setup so the NotificationExt API is available to the sender closure.
        .plugin(tauri_plugin_notification::init())
        // Custom `seekermail://` scheme for OAuth deep-link callbacks (T015/T064).
        .plugin(tauri_plugin_deep_link::init())
        // WB-22: a destroyed detached workbench window tells the survivors to refresh.
        .on_window_event(commands::workbench::on_window_event)
        .setup(move |app| {
            use tauri::Manager;

            // Point the bundled-model resource dir at the packaged app's resources
            // (where `model.onnx` + `model.onnx_data` + `tokenizer.json` ship). This
            // needs the AppHandle's `resource_dir()`, so it happens here rather than
            // in `Paths::resolve`. Without it the real bge-m3 embedder can't find its
            // model and silently falls back to the offline embedder. A set
            // `SEEKERMAIL_RESOURCE_DIR` dev override wins and is left untouched.
            let mut paths = paths;
            if std::env::var_os("SEEKERMAIL_RESOURCE_DIR").is_none() {
                match app.path().resource_dir() {
                    Ok(dir) => paths.resources = dir.join("resources"),
                    Err(e) => tracing::warn!(
                        error = %e,
                        "resource_dir unavailable; embedding model falls back to the user models dir"
                    ),
                }
            }

            // Bootstrap storage + shared state. A migration failure here would
            // otherwise bubble out of `setup`, make Tauri `panic!`, and (release
            // builds are `panic = "abort"`) crash the app on launch with no message
            // — macOS shows only "Reopen", and reinstalling can't fix it because the
            // database survives an uninstall. Forward-only migrations mean a normal
            // upgrade never fails here; a downgrade, a DB written by a build whose
            // migrations later changed, or on-disk damage can. In that one case let
            // the user back up & reset the local database (moved aside, never
            // deleted) and retry, or quit — never crash. (See `recovery`.)
            let mut db_reset_done = false;
            let (state, mail_rx, embed_rx, pipeline_rx) = loop {
                let emitter = events::Emitter::new(app.handle().clone());
                match tauri::async_runtime::block_on(AppState::bootstrap(paths.clone(), emitter)) {
                    Ok(parts) => break parts,
                    // Offer recovery once. If a freshly reset database still fails to
                    // migrate, the migrations themselves are broken — fall through to
                    // the generic error path rather than loop on the dialog forever.
                    Err(error::AppError::DbMigration(detail)) if !db_reset_done => {
                        tracing::error!(
                            detail = %detail,
                            "startup migration failed; offering database recovery"
                        );
                        if recovery::prompt_and_reset_database(&paths, &detail) {
                            db_reset_done = true;
                            continue;
                        }
                        // User chose to quit; exit cleanly without panicking.
                        std::process::exit(0);
                    }
                    Err(e) => {
                        return Err(Box::<dyn std::error::Error>::from(e.to_string()));
                    }
                }
            };

            // Background workers run on the Tauri/Tokio runtime. Several of these
            // spawn fns call `tokio::spawn` directly, which panics ("there is no
            // reactor running") when invoked from the synchronous setup hook —
            // it runs outside any runtime context. Enter the runtime via block_on
            // so the ambient runtime exists; the spawned tasks outlive this call
            // on the global multi-thread runtime. (Matches the block_on already
            // used for bootstrap/scheduler in this same hook.)
            let worker_state = state.clone();
            tauri::async_runtime::block_on(async move {
                // Ingest worker: parse → sanitise → persist (T023).
                imap::parser::spawn_parse_worker(mail_rx, worker_state.clone());

                // B3 embedding worker: chunk → embed → vector upsert (T031).
                embedding::queue::start_worker(embed_rx, worker_state.clone());

                // E2/E3 AI pipeline worker: consumes ingest jobs, routes by auth
                // level, generates drafts / queues auto-sends (T082/T085).
                ai::pipeline::worker::start_pipeline_worker(pipeline_rx, worker_state.clone());

                // E3 delayed-send queue scan: delivers due auto-replies, doubles
                // as restart recovery for rows queued before a shutdown (T085).
                ai::pipeline::e3_send_queue::start_send_queue_worker(worker_state.clone());
            });

            // T083: install the OS-notification sender now that an AppHandle
            // exists. The notifier itself only ever emits counts, no content.
            {
                use tauri_plugin_notification::NotificationExt;
                let handle = app.handle().clone();
                state
                    .notifier
                    .set_sender(Box::new(move |title: &str, body: &str| {
                        if let Err(e) = handle
                            .notification()
                            .builder()
                            .title(title)
                            .body(body)
                            .show()
                        {
                            tracing::warn!(error = %e, "os notification failed");
                        }
                    }));
            }

            // Deep-link OAuth callbacks (T015/T064): the OS routes the provider's
            // `seekermail://oauth/...` redirect here. Route by path — the
            // account-mail grant (`/oauth/callback`) and the recommended-provider
            // grant (`/oauth/recommended`) each get their own event; the matching
            // frontend wizard listens and forwards code+state to its `complete_*`
            // command (where the CSRF check lives). Registration is best-effort: a
            // dev build whose scheme isn't installed simply receives no links, and
            // both wizards also accept a manual code paste as a fallback.
            {
                use tauri::Emitter as _;
                use tauri_plugin_deep_link::DeepLinkExt;

                #[cfg(desktop)]
                let _ = app.deep_link().register("seekermail");

                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        let raw = url.as_str();
                        if let Some(cb) = ai::recommended::parse_recommended_callback(raw) {
                            let _ = handle.emit(ai::recommended::OAUTH_CALLBACK_EVENT, cb);
                        } else if let Some(cb) = account::oauth::parse_mail_callback(raw) {
                            let _ = handle.emit(account::oauth::MAIL_OAUTH_CALLBACK_EVENT, cb);
                        }
                    }
                });
            }

            // Sync scheduler: one poll task per active account + backfill resume (T021/T022).
            let scheduler =
                tauri::async_runtime::block_on(imap::SyncScheduler::start(state.clone()));

            // BYO AI adapters (T059/T060/T062/T063): per-account factories.
            // ADR-0004 — every adapter talks only to the endpoint the user
            // configured; no SeekerMail server is ever in the path.
            {
                use std::sync::Arc;

                use ai::providers::anthropic::AnthropicClient;
                use ai::providers::local_onnx::LocalOnnxClient;
                use ai::providers::ollama::OllamaClient;
                use ai::providers::openai::OpenAiClient;
                use ai::AiProviderClient;
                use types::AiProvider;

                let kc = state.keychain.clone();
                state.ai.register_factory(
                    AiProvider::Openai,
                    Arc::new(move |cfg: &ai::AccountAiConfig| {
                        OpenAiClient::from_config(cfg, kc.clone())
                            .map(|c| c as Arc<dyn AiProviderClient>)
                    }),
                );
                let kc = state.keychain.clone();
                state.ai.register_factory(
                    AiProvider::Anthropic,
                    Arc::new(move |cfg: &ai::AccountAiConfig| {
                        AnthropicClient::from_config(cfg, kc.clone())
                            .map(|c| c as Arc<dyn AiProviderClient>)
                    }),
                );
                state.ai.register_factory(
                    AiProvider::Ollama,
                    Arc::new(|cfg: &ai::AccountAiConfig| {
                        OllamaClient::from_config(cfg).map(|c| c as Arc<dyn AiProviderClient>)
                    }),
                );
                let local_paths = paths.clone();
                state.ai.register_factory(
                    AiProvider::LocalOnnx,
                    Arc::new(move |cfg: &ai::AccountAiConfig| {
                        LocalOnnxClient::from_config(cfg, &local_paths)
                            .map(|c| c as Arc<dyn AiProviderClient>)
                    }),
                );
            }

            // E5 style-profile refresh worker: re-learns stale profiles on a
            // 30-day cadence (T075).
            ai::style::start_refresh_worker(state.clone());

            // E6 draft expiry sweep (T080): runs at startup, then every 30 min.
            ai::draft::expiry::start_expiry_worker(state.clone());

            // E7 audit-log retention sweep (T088): daily policy purge.
            ai::audit::retention::start_retention_worker(state.clone());

            // F5 recovery loop (T067): probe cooled-down providers and drain
            // the hold queue when a provider comes back.
            {
                let fallback = state.fallback.clone();
                tauri::async_runtime::spawn(async move {
                    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
                    loop {
                        tick.tick().await;
                        fallback.run_recovery_tick().await;
                    }
                });
            }

            // I3 query expiry sweep (T097): auto-expire overdue non-T4 queries and
            // post the merged daily T4 reminder. Runs at startup, then every 15 min.
            {
                let state = state.clone();
                tauri::async_runtime::spawn(async move {
                    let mut tick = tokio::time::interval(std::time::Duration::from_secs(900));
                    loop {
                        tick.tick().await;
                        if let Err(e) =
                            ai::pipeline::query_expiry::run_query_expiry_check(&state).await
                        {
                            tracing::error!(error = %e, "query expiry check failed");
                        }
                    }
                });
            }

            // Privacy defaults: tracking protection ON for first runs (T051 §6).
            tauri::async_runtime::block_on(commands::settings::ensure_privacy_defaults(&state))
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

            // T091: heal the single-primary-account invariant before the UI loads,
            // so a 0- or ≥2-primary database (corruption / aborted migration) is
            // repaired silently instead of confusing the Agents page.
            tauri::async_runtime::block_on(account::AccountService::heal_primary(&state))
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

            // FOUC guard (T050 §6, analysis 25): hand the persisted theme and UI
            // scale to the webview as globals before React mounts. The boot scripts
            // read them; main.tsx re-reads the settings over IPC as the late fallback.
            let theme = tauri::async_runtime::block_on(commands::settings::initial_theme(&state));
            let font_scale =
                tauri::async_runtime::block_on(commands::settings::initial_font_scale(&state));
            if let Some(win) = app.get_webview_window("main") {
                // `theme` is constrained to light|dark|system — safe to embed.
                let _ = win.eval(format!("window.__INITIAL_THEME__ = \"{theme}\";"));
                // `font_scale` is a clamped f64 — safe to embed as a numeric literal.
                let _ = win.eval(format!("window.__INITIAL_FONT_SCALE__ = {font_scale};"));
            }

            app.manage(state);
            app.manage(scheduler);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Reference each command by the full path to the module where
            // `#[tauri::command]` is applied, so `generate_handler!` finds its
            // generated `__cmd__*` helpers (a re-export like `commands::ping`
            // does not carry those helper items).
            commands::system::ping,
            // ── Workbench windows (WB-12, Model S — 02 §3 Module W) ──────────
            // T2 (WB-12): verify with `cargo build` on the Mac (dev/22 §A).
            commands::workbench::workbench_open_window,
            commands::workbench::workbench_close_window,
            commands::workbench::workbench_focus_window,
            commands::workbench::workbench_list_windows,
            // ── Accounts (T013) ──────────────────────────────────────────────
            commands::accounts::list_accounts,
            commands::accounts::get_account,
            commands::accounts::create_account,
            commands::accounts::update_account,
            commands::accounts::delete_account,
            // ── SeekerMail ID identity (A6, decoupled from mailboxes) ────────
            commands::identity::sign_out_seekermail,
            commands::identity::get_seekermail_id,
            commands::identity::set_marketing_consent,
            commands::identity::begin_google_signin,
            commands::identity::complete_google_signin,
            commands::accounts::update_account_password,
            commands::accounts::enable_account,
            commands::accounts::disable_account,
            commands::accounts::set_primary_account,
            // ── Connection probe (T014) ──────────────────────────────────────
            commands::accounts::verify_account_connection,
            // ── OAuth (T015/T018) ────────────────────────────────────────────
            commands::accounts::begin_oauth_flow,
            commands::accounts::complete_oauth_flow,
            commands::accounts::reauth_account,
            // ── Knowledge depth + sampling (T016) ────────────────────────────
            commands::accounts::sample_mailbox,
            commands::accounts::set_knowledge_depth,
            // ── Disk usage (T020) ────────────────────────────────────────────
            commands::accounts::get_account_disk_usage,
            // ── Sync control (T021) ──────────────────────────────────────────
            commands::accounts::trigger_sync,
            commands::accounts::get_sync_state,
            // ── Backfill (T022) ──────────────────────────────────────────────
            commands::accounts::get_backfill_status,
            commands::accounts::pause_backfill,
            commands::accounts::resume_backfill,
            // ── Attachments (T025/T026) ──────────────────────────────────────
            commands::accounts::download_attachment,
            commands::accounts::get_attachments_for_mail,
            commands::accounts::open_attachment,
            commands::accounts::reveal_attachment,
            commands::accounts::get_attachment_local_path,
            // ── Attachment text extraction + index (T108/T109) ───────────────
            commands::extraction::start_attachment_extraction_backfill,
            commands::extraction::build_attachment_index,
            // ── Shell / external links ───────────────────────────────────────
            commands::shell::open_external_url,
            // ── Tracker / remote images (T029) ───────────────────────────────
            commands::mail::get_tracker_info,
            commands::mail::allow_remote_images,
            // Inline (cid:) image resolution + privacy-hardened remote-image fetch.
            commands::mail::get_inline_images,
            commands::mail::fetch_remote_image,
            // ── Mail-list read backend (G2/G3) ───────────────────────────────
            commands::mail::list_threads,
            commands::mail::list_mails,
            commands::mail::get_mail,
            commands::mail::set_mail_read,
            commands::mail::set_mail_starred,
            commands::mail::archive_mail,
            commands::mail::delete_mail,
            commands::mail::set_mail_spam,
            commands::mail::restore_mail,
            // ── Search (T032/T033/T035) ──────────────────────────────────────
            commands::search::keyword_search,
            commands::search::semantic_search,
            commands::search::get_search_history,
            commands::search::list_saved_searches,
            commands::search::save_search,
            commands::search::delete_saved_search,
            // ── Attachment-hit search (T110) ─────────────────────────────────
            commands::search::search_with_attachments,
            // GTE index stats + topic breakdown (Repository / GTE pages)
            commands::gte::get_gte_stats,
            commands::gte::get_topic_breakdown,
            commands::gte::list_knowledge_entries,
            commands::memory::build_thread_summaries,
            // ── Compose / send (T043) ────────────────────────────────────────
            commands::mail::send_mail,
            commands::mail::cancel_send,
            // ── Drafts (T045) ────────────────────────────────────────────────
            commands::draft::save_draft,
            commands::draft::get_draft,
            commands::draft::delete_draft,
            // ── Settings (T050/T051) ─────────────────────────────────────────
            commands::settings::get_setting,
            commands::settings::set_setting,
            commands::settings::set_global_pref,
            commands::settings::apply_privacy_policy,
            // ── Export (T052) ────────────────────────────────────────────────
            commands::export::start_export,
            commands::export::cancel_export,
            commands::export::open_export_output,
            // ── Wipe / reindex / sync range (T053) ───────────────────────────
            commands::wipe::preview_wipe,
            commands::wipe::start_wipe,
            commands::reindex::start_reindex,
            commands::reindex::cancel_reindex,
            commands::sync_range::preview_sync_range,
            commands::sync_range::update_sync_range,
            // ── BYO AI settings + provider probe (T059, Module H) ────────────
            commands::ai::get_account_ai_settings,
            commands::ai::update_account_ai_settings,
            commands::ai::verify_ai_provider,
            // ── F4 provider matrix (T065) ────────────────────────────────────
            commands::ai::get_provider_matrix,
            commands::ai::update_provider_matrix,
            commands::ai::reset_provider_matrix_to_defaults,
            commands::ai::batch_update_provider_matrix,
            // ── Role analysis (T070 legal / T072 sales) ──────────────────────
            commands::ai_roles::analyze_legal_risk,
            commands::ai_roles::analyze_sales_context,
            // ── E1 manual reply generation (T077, Module E) ──────────────────
            commands::ai::request_ai_reply,
            commands::ai::regenerate_draft,
            // ── E6 draft queue (T080) + approve/cancel send (T090) ───────────
            commands::ai::list_pending_drafts,
            commands::ai::get_ai_draft,
            commands::ai::update_draft_body,
            commands::ai::approve_draft,
            commands::ai::discard_draft,
            commands::ai::cancel_draft_send,
            // ── E7 audit log (T088) ──────────────────────────────────────────
            commands::ai::list_ai_decisions,
            commands::ai::get_ai_decisions_summary,
            commands::ai::export_ai_decisions,
            // ── F5 offline fallback (T067) ───────────────────────────────────
            commands::ai::set_ai_disabled,
            // ── Style learning (T075, E5) ────────────────────────────────────
            commands::style::trigger_style_learning,
            // ── Provider config UI surface (T068) ────────────────────────────
            commands::ai::scan_local_providers,
            commands::ai::list_ollama_models,
            commands::ai::list_cloud_models,
            commands::ai::list_configured_providers,
            // ── Recommended provider (T064, F3) ──────────────────────────────
            commands::ai_recommended::get_recommended_providers,
            commands::ai_recommended::get_ai_setup_status,
            commands::ai_recommended::confirm_ai_disclosure,
            commands::ai_recommended::clear_conservative_quota,
            commands::ai_recommended::begin_recommended_oauth,
            commands::ai_recommended::complete_recommended_oauth,
            commands::ai_recommended::revoke_recommended_provider,
            // ── Data-flow disclosure, AI section (T069) ──────────────────────
            commands::data_flow::get_data_flow_ai_routing,
            // ── Agent-IM / TEAM channel (T092) ───────────────────────────────
            commands::im::post_im_message,
            commands::im::list_im_messages,
            commands::im::mark_im_message_read,
            commands::im::mark_im_channel_read,
            commands::im::count_pending_queries,
            commands::im::count_team_unread,
            // ── Agent presence / identity (T094) ─────────────────────────────
            commands::agents::get_agent_statuses,
            // ── Proactive queries (T096/T099) ────────────────────────────────
            commands::queries::list_pending_queries,
            commands::queries::answer_query,
            commands::queries::skip_query,
            // ── Risk events (T071, Module E) ─────────────────────────────────
            commands::risk::list_risk_events,
            commands::risk::resolve_risk_event,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
