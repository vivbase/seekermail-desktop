//! Startup database recovery — keeps a migration failure from crashing the app.
//!
//! `AppState::bootstrap` applies the embedded SQLite migrations at launch. If that
//! fails — a downgrade, a database written by a build whose migrations later
//! changed (checksum drift), or on-disk damage — the setup hook would return
//! `Err`, Tauri would `panic!`, and (release builds are `panic = "abort"`) the
//! process aborts instantly. macOS then shows only its "application quit
//! unexpectedly / Reopen" dialog with no explanation, and because uninstalling the
//! app leaves the database in place, reinstalling does not fix it.
//!
//! Instead, [`prompt_and_reset_database`] shows a native dialog that lets the user
//! back up & reset the local database (the file is moved aside, never deleted) so
//! the next open starts from a clean schema, or quit. Forward-only migrations
//! (see [`crate::storage`]) mean a normal upgrade never reaches this path.

use std::path::{Path, PathBuf};

use crate::config::Paths;

/// First (default) button label; also the value matched in the dialog result.
const RESET_LABEL: &str = "Back Up & Reset";
/// Second button label.
const QUIT_LABEL: &str = "Quit";

/// Show the recovery dialog. On "Back Up & Reset", move the database (and its
/// `-wal` / `-shm` sidecars) aside and return `true` so the caller retries
/// bootstrap against a fresh file. On "Quit", return `false`. If the move itself
/// fails there is nothing left to try, so a final dialog is shown and the process
/// exits.
pub fn prompt_and_reset_database(paths: &Paths, detail: &str) -> bool {
    if !user_chose_reset(detail) {
        return false;
    }
    match back_up_database_aside(&paths.db) {
        Ok(moved_to) => {
            tracing::warn!(
                backup = %moved_to.display(),
                "local database reset by user; previous file preserved"
            );
            true
        }
        Err(e) => {
            tracing::error!(error = %e, "could not move the database aside for reset");
            show_reset_failed(&paths.db, &e.to_string());
            std::process::exit(1);
        }
    }
}

/// Render the recovery prompt and report whether the user picked "Back Up & Reset".
fn user_chose_reset(detail: &str) -> bool {
    let body = format!(
        "SeekerMail couldn't open your local database.\n\n\
         This can happen after switching app versions, or if the database file was \
         damaged. You can back up the current database and start fresh: your existing \
         file is moved aside (never deleted), and your accounts can be added again \
         afterwards.\n\n\
         Technical detail: {detail}"
    );
    let result = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("SeekerMail can't open your data")
        .set_description(body)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            RESET_LABEL.to_owned(),
            QUIT_LABEL.to_owned(),
        ))
        .show();
    matches!(result, rfd::MessageDialogResult::Custom(label) if label == RESET_LABEL)
}

/// Final notice when even the reset move fails; the user must act manually.
fn show_reset_failed(db: &Path, detail: &str) {
    let body = format!(
        "SeekerMail couldn't reset the local database automatically.\n\n\
         Please quit, move this file aside manually, then reopen SeekerMail:\n\n{}\n\n\
         Technical detail: {detail}",
        db.display()
    );
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Couldn't reset the database")
        .set_description(body)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Move `db` and its SQLite sidecars to timestamped `*.broken-<ts>` siblings so the
/// next open creates a fresh database. Returns the new path of the main file. The
/// old data is preserved, never deleted (data-sovereignty: the user can restore it
/// by renaming it back).
fn back_up_database_aside(db: &Path) -> std::io::Result<PathBuf> {
    let suffix = format!(".broken-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    // Sidecars first (best-effort) — they are meaningless without the main file.
    for ext in ["-wal", "-shm"] {
        let side = append(db, ext);
        if side.exists() {
            let _ = std::fs::rename(&side, append(&side, &suffix));
        }
    }
    let target = append(db, &suffix);
    std::fs::rename(db, &target)?;
    Ok(target)
}

/// Append a raw string to a path's full filename (not as a new path component):
/// e.g. `seekermail.db` + `-wal` → `seekermail.db-wal`.
fn append(path: &Path, s: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(s);
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_extends_full_filename() {
        let p = Path::new("/data/seekermail.db");
        assert_eq!(append(p, "-wal"), Path::new("/data/seekermail.db-wal"));
        assert_eq!(
            append(p, ".broken-x"),
            Path::new("/data/seekermail.db.broken-x")
        );
    }

    #[test]
    fn back_up_moves_main_and_sidecars_and_frees_the_path() {
        let dir = std::env::temp_dir().join(format!("sk-recovery-{}", crate::util::new_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("seekermail.db");
        std::fs::write(&db, b"main").unwrap();
        std::fs::write(append(&db, "-wal"), b"wal").unwrap();
        std::fs::write(append(&db, "-shm"), b"shm").unwrap();

        let moved = back_up_database_aside(&db).unwrap();

        assert!(!db.exists(), "main file should have been moved aside");
        assert!(moved.exists(), "backup of the main file should exist");
        assert!(
            !append(&db, "-wal").exists(),
            "wal sidecar should have been moved"
        );
        assert!(
            !append(&db, "-shm").exists(),
            "shm sidecar should have been moved"
        );
        // A fresh database can now be created at the original path.
        std::fs::write(&db, b"fresh").unwrap();
        assert!(db.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
