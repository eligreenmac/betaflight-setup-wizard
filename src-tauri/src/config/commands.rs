//! Tauri commands for workstream `config`. Signatures are a contract with the UI
//! (src/api/commands.ts, docs/ARCHITECTURE.md §3 and §5.1).
//!
//! The FC-facing commands borrow the app-wide link from device's
//! [`ConnectionManager`] (`with_link`, ARCHITECTURE §4.1) on a blocking thread, so
//! they never hold the async runtime: no open port → `NOT_CONNECTED`; a motor test, RC
//! stream or another command holding the port → `PORT_BUSY {heldBy: "APP"}`. The link
//! is held for the whole operation (apply/rollback: backup → batch → save → reboot →
//! reconnect), so nothing else can talk to the FC in between.
//!
//! The logic lives in the plain functions below (generic over [`LinkSource`]) and is
//! unit-tested against `MockFc`; the `#[tauri::command]` wrappers only add the Tauri
//! state, the backups folder and the `config://progress` events.

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::backup::BackupStore;
use super::session;
use crate::device::{ConnectionManager, FcLink};
use crate::error::{AppError, AppResult};
use crate::model::{
    ApplyResult, Backup, CliScript, ConfigProgress, DroneModel, FcSerialPort, FirmwareCatalog,
    OsdLayout, OsdVideoSystem, RestoreReport, VerifyReport,
};

/// `config://progress`, payload [`ConfigProgress`] (stages BACKUP, SENDING, SAVING,
/// REBOOTING, RECONNECTED) for `config_apply`, `config_rollback` and `config_restore`.
pub const PROGRESS_EVENT: &str = "config://progress";

/// Where the FC link comes from: device's [`ConnectionManager`] in the app, a mock in
/// tests. Same semantics as `ConnectionManager::with_link`.
pub trait LinkSource {
    fn with_link<T>(&self, f: impl FnOnce(&mut dyn FcLink) -> AppResult<T>) -> AppResult<T>;
}

impl LinkSource for ConnectionManager {
    fn with_link<T>(&self, f: impl FnOnce(&mut dyn FcLink) -> AppResult<T>) -> AppResult<T> {
        ConnectionManager::with_link(self, f)
    }
}

// ------------------------------------------------------------------ logic (testable)

/// Backup folder key `backups/<boardUid>/`: the link's cached MSP_UID in the CLI
/// `mcu_id` form (`FcInfo.uid`, ARCHITECTURE §4.1; no I/O), `unknown` if the FC did not
/// report one (§5.1).
fn board_uid(link: &dyn FcLink) -> String {
    link.uid().unwrap_or_else(|| "unknown".into())
}

/// C2: the FC's serial ports (`serial`, dialect-specific identifiers).
pub fn read_ports(links: &impl LinkSource) -> AppResult<Vec<FcSerialPort>> {
    links.with_link(|l| session::read_ports(l))
}

/// C4: `diff all` -> `backups/<boardUid>/<UTC stamp>.txt`.
pub fn backup(links: &impl LinkSource, store: &BackupStore) -> AppResult<Backup> {
    links.with_link(|l| {
        let uid = board_uid(l);
        session::backup(l, store, &uid)
    })
}

/// C5: read-only plan against the connected FC.
pub fn plan(links: &impl LinkSource, model: &DroneModel) -> AppResult<CliScript> {
    links.with_link(|l| session::plan(l, model))
}

/// C5: backup -> batch -> save -> reboot -> reconnect (`session::apply`).
pub fn apply(
    links: &impl LinkSource,
    store: &BackupStore,
    script: &CliScript,
    progress: &mut dyn FnMut(ConfigProgress),
) -> AppResult<ApplyResult> {
    // Checked before the port is borrowed: a bad script sends nothing at all.
    super::guard::validate_script(&script.commands)?;
    links.with_link(|l| {
        let uid = board_uid(l);
        session::apply(l, script, store, &uid, progress)
    })
}

/// C6: read back and compare with `model`.
pub fn verify(links: &impl LinkSource, model: &DroneModel) -> AppResult<VerifyReport> {
    links.with_link(|l| session::verify(l, model))
}

/// C6: restore `backup`. The backup is looked up again by `id` in the store; the `path`
/// sent by the UI is never opened, so only files inside `backups/` can be replayed.
pub fn rollback(
    links: &impl LinkSource,
    store: &BackupStore,
    backup: &Backup,
    progress: &mut dyn FnMut(ConfigProgress),
) -> AppResult<ApplyResult> {
    restore(links, store, &backup.id, progress)
}

/// C6: restore by id (`config_list_backups`): safety backup, then the rollback path.
/// Refuses (CONFIG, nothing sent) a backup taken on another board (both UIDs
/// known and different) or on another firmware line (`session::rollback`).
pub fn restore(
    links: &impl LinkSource,
    store: &BackupStore,
    backup_id: &str,
    progress: &mut dyn FnMut(ConfigProgress),
) -> AppResult<ApplyResult> {
    let target = store.find(backup_id)?;
    // Checked before the port is borrowed: a tampered or foreign file sends nothing.
    super::guard::replay_lines(&std::fs::read_to_string(&target.path)?)?;
    links.with_link(|l| {
        let uid = board_uid(l);
        let known = |u: &str| !u.is_empty() && u != "unknown";
        if known(&uid) && known(&target.board_uid) && !uid.eq_ignore_ascii_case(&target.board_uid) {
            return Err(AppError::Config(format!(
                "backup {} was taken on another board (UID {}), the connected FC is {}; nothing was changed",
                target.id, target.board_uid, uid
            )));
        }
        session::rollback(l, &target, store, &uid, progress)
    })
}

/// Restore after flash (ARCHITECTURE §5.2): the backup is found, read and checked before the
/// port is borrowed (`BACKUP_UNREADABLE` / `BACKUP_NOT_DIFF_ALL`, nothing sent); no link →
/// `BOARD_NOT_RECONNECTED` without a stage; `PORT_BUSY` passes through unchanged.
pub fn restore_after_flash(
    links: &impl LinkSource,
    store: &BackupStore,
    backup_id: &str,
    progress: &mut dyn FnMut(ConfigProgress),
) -> AppResult<RestoreReport> {
    use super::restore;
    use crate::error::ConfigErrorReason;
    let target = store.find(backup_id).map_err(|e| {
        AppError::restore_rejected(
            ConfigErrorReason::BackupUnreadable,
            e.to_string(),
            backup_id,
            None,
            None,
        )
    })?;
    let prepared = restore::prepare(&target, std::fs::read_to_string(&target.path))?;
    links
        .with_link(|l| {
            let uid = board_uid(l);
            restore::run(l, &prepared, &uid, progress)
        })
        .map_err(|e| match e {
            AppError::NotConnected => AppError::restore_rejected(
                ConfigErrorReason::BoardNotReconnected,
                "the FC is not connected; reconnect it and retry the restore",
                backup_id,
                None,
                None,
            ),
            e => e,
        })
}

// ------------------------------------------------------------------ Tauri wrappers

/// `<app_data_dir>/backups`
fn backups_root<R: Runtime>(app: &AppHandle<R>) -> AppResult<PathBuf> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Io(format!("app data dir: {e}")))?;
    Ok(base.join("backups"))
}

/// Run `f` on a blocking thread with the connection manager, the backup store and an
/// emitter for `config://progress`.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&ConnectionManager, &BackupStore, &mut dyn FnMut(ConfigProgress)) -> AppResult<T>
        + Send
        + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = BackupStore::new(backups_root(&app)?);
        let mgr = app.state::<ConnectionManager>();
        let mut emit = |p: ConfigProgress| {
            let _ = app.emit(PROGRESS_EVENT, p);
        };
        f(&mgr, &store, &mut emit)
    })
    .await
    .map_err(|e| AppError::Config(format!("config worker failed: {e}")))?
}

/// Read the FC's serial ports (names, firmware identifiers, current functions).
#[tauri::command]
pub async fn config_read_ports(app: AppHandle) -> AppResult<Vec<FcSerialPort>> {
    blocking(app, |m, _, _| read_ports(m)).await
}

/// Save `diff all` from the connected FC to `backups/<boardUid>/`.
#[tauri::command]
pub async fn config_backup(app: AppHandle) -> AppResult<Backup> {
    blocking(app, |m, store, _| backup(m, store)).await
}

/// PREVIEW ONLY. Pure generation (no FC I/O): DroneModel + caller-supplied ports -> CLI
/// script. It assumes default baud rates and free aux slots and cannot check SPI receiver
/// hardware, so its output must never be applied: `config_apply` takes the script from
/// `config_plan`.
#[tauri::command]
pub async fn config_generate(model: DroneModel, ports: Vec<FcSerialPort>) -> AppResult<CliScript> {
    super::generate(&model, &ports)
}

/// Binding phrase -> 6-byte ExpressLRS UID (first 6 bytes of
/// MD5(`-DMY_BINDING_PHRASE="<phrase>"`)). Pure. The phrase is never stored or logged;
/// the UI puts only the returned UID in `receiver.elrsUid`.
#[tauri::command]
pub async fn config_elrs_uid(phrase: String) -> AppResult<[u8; 6]> {
    // Hashed exactly as typed (the ELRS tools do not trim either).
    if phrase.is_empty() {
        return Err(AppError::Config("binding phrase is empty".into()));
    }
    Ok(super::md5::elrs_uid(&phrase))
}

/// Config's default OSD layout for a video system on a firmware line (every element the
/// line has, with x/y and default visibility). Pure, no FC I/O; UNSUPPORTED_FIRMWARE
/// outside 4.4.x / 4.5.x / 2025.12.x / 2026.6.x.
#[tauri::command]
pub async fn config_osd_default_layout(
    video_system: OsdVideoSystem,
    firmware_version: String,
) -> AppResult<OsdLayout> {
    let dialect = super::firmware::Dialect::from_version(&firmware_version)?;
    Ok(super::osd::default_layout(video_system, dialect))
}

/// Per-firmware catalog (beepers, DShot beacon events, OSD elements, stable mode names) so
/// the UI does not hard-code them. Pure, no FC I/O; UNSUPPORTED_FIRMWARE outside the
/// supported lines.
#[tauri::command]
pub async fn config_firmware_catalog(firmware_version: String) -> AppResult<FirmwareCatalog> {
    let dialect = super::firmware::Dialect::from_version(&firmware_version)?;
    Ok(super::catalog::firmware_catalog(dialect))
}

/// Read-only plan against the connected FC: reads `version`, `serial`, `dump all`
/// (firmware gate, SPI ELRS detection, aux slots, available settings) and returns the
/// script plus `cleared` ports. Writes nothing. This is the script the UI applies.
#[tauri::command]
pub async fn config_plan(app: AppHandle, model: DroneModel) -> AppResult<CliScript> {
    blocking(app, move |m, _, _| plan(m, &model)).await
}

/// Apply: `diff all` backup -> `batch start` + commands + `save` (batch still active) ->
/// reboot -> reconnect. Resolves only after reconnect; emits `config://progress`.
/// The script is checked first against the generator's verbs (`guard`): a line that
/// would persist, reboot, wipe or leave the batch is a CONFIG error and nothing is sent.
#[tauri::command]
pub async fn config_apply(app: AppHandle, script: CliScript) -> AppResult<ApplyResult> {
    blocking(app, move |m, store, emit| apply(m, store, &script, emit)).await
}

/// After reconnect, read back settings and compare to `model`.
#[tauri::command]
pub async fn config_verify(app: AppHandle, model: DroneModel) -> AppResult<VerifyReport> {
    blocking(app, move |m, _, _| verify(m, &model)).await
}

/// Restore a backup: safety backup, then replay its `diff all` (`defaults nosave` +
/// lines) + `save` + reconnect; emits `config://progress`. Which backup lines are
/// replayed is defined by `guard::replay_lines`; anything else in the file is a CONFIG
/// error and nothing is sent. The backup is resolved by `id` inside the backups folder.
#[tauri::command]
pub async fn config_rollback(app: AppHandle, backup: Backup) -> AppResult<ApplyResult> {
    blocking(app, move |m, store, emit| rollback(m, store, &backup, emit)).await
}

/// All backups on disk, newest first (all boards).
#[tauri::command]
pub async fn config_list_backups<R: Runtime>(app: AppHandle<R>) -> AppResult<Vec<Backup>> {
    BackupStore::new(backups_root(&app)?).list()
}

/// Absolute path of the backups root folder (UI opens it with the opener plugin).
#[tauri::command]
pub async fn config_backups_dir<R: Runtime>(app: AppHandle<R>) -> AppResult<String> {
    let root = backups_root(&app)?;
    std::fs::create_dir_all(&root)?;
    Ok(root.to_string_lossy().into_owned())
}

/// Restore by id (from config_list_backups): safety backup, then the rollback path.
#[tauri::command]
pub async fn config_restore(app: AppHandle, backup_id: String) -> AppResult<ApplyResult> {
    blocking(app, move |m, store, emit| {
        restore(m, store, &backup_id, emit)
    })
    .await
}

/// Restore a `diff all` backup after a firmware flash, across firmware lines
/// (ARCHITECTURE.md §5.2).
#[tauri::command]
pub async fn config_restore_after_flash(
    app: AppHandle,
    backup_id: String,
) -> AppResult<RestoreReport> {
    blocking(app, move |m, store, emit| {
        restore_after_flash(m, store, &backup_id, emit)
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, TryLockError};
    use std::time::Duration;

    use super::*;
    use crate::config::firmware::Dialect;
    use crate::config::mock::MockFc;
    use crate::device::manager::{BoxLink, Lister, Opener, PortIdentity};
    use crate::device::motor::fake::{CliBackend, FakeFc};
    use crate::device::Connection;
    use crate::error::PortHolder;
    use crate::model::ConfigStage;

    /// Stand-in for the connection manager: one slot, `NOT_CONNECTED` when empty and
    /// `PORT_BUSY {APP}` while borrowed (same rules as `ConnectionManager::with_link`).
    struct MockSource(Mutex<Option<MockFc>>);

    impl MockSource {
        fn new(fc: MockFc) -> Self {
            MockSource(Mutex::new(Some(fc)))
        }
        fn fc<T>(&self, f: impl FnOnce(&mut MockFc) -> T) -> T {
            f(self.0.lock().unwrap().as_mut().unwrap())
        }
    }

    impl LinkSource for MockSource {
        fn with_link<T>(&self, f: impl FnOnce(&mut dyn FcLink) -> AppResult<T>) -> AppResult<T> {
            let mut g = match self.0.try_lock() {
                Ok(g) => g,
                Err(TryLockError::WouldBlock) => {
                    return Err(AppError::PortBusy {
                        port: "MOCK".into(),
                        held_by: PortHolder::App,
                    })
                }
                Err(TryLockError::Poisoned(p)) => p.into_inner(),
            };
            match g.as_mut() {
                Some(fc) => f(fc),
                None => Err(AppError::NotConnected),
            }
        }
    }

    fn model(name: &str) -> DroneModel {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/models/{name}.json"));
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn store(tag: &str) -> BackupStore {
        let dir = std::env::temp_dir().join(format!("bfsw-cmd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BackupStore::new(dir)
    }

    const UID: &str = "000000000000000100000002"; // fixture `mcu_id` = MockFc::uid()

    #[test]
    fn read_ports_and_backup_use_the_borrowed_link() {
        for d in Dialect::ALL {
            let src = MockSource::new(MockFc::load(d.as_str(), "speedybeef405v4"));
            let ports = read_ports(&src).unwrap();
            let expected = src.fc(|fc| session::read_ports(fc).unwrap());
            assert_eq!(ports.len(), expected.len());
            assert!(ports.iter().any(|p| p.name == "UART1"), "{d:?}");

            let st = store(&format!("bk-{}", d.as_str()));
            let bk = backup(&src, &st).unwrap();
            assert_eq!(bk.board_uid, UID);
            assert!(bk.id.starts_with(&format!("{UID}/")), "{}", bk.id);
            assert!(PathBuf::from(&bk.path).starts_with(st.root().join(UID)));
            assert!(std::fs::read_to_string(&bk.path)
                .unwrap()
                .contains("defaults nosave"));
            // Read-only: nothing that writes or leaves the CLI.
            src.fc(|fc| {
                assert_eq!(fc.reboots, 0);
                assert!(!fc.log.iter().any(|l| l.starts_with("set ") || l == "save"));
            });
            let _ = std::fs::remove_dir_all(st.root());
        }
    }

    #[test]
    fn unknown_uid_goes_to_the_unknown_folder() {
        let mut fc = MockFc::load("4.5", "speedybeef405v4");
        fc.mcu_id.clear(); // link.uid() == None (MSP_UID unsupported)
        let src = MockSource::new(fc);
        let st = store("bk-unknown");
        let bk = backup(&src, &st).unwrap();
        assert_eq!(bk.board_uid, "unknown");
        let _ = std::fs::remove_dir_all(st.root());
    }

    #[test]
    fn plan_apply_verify_restore_with_progress_events() {
        for d in Dialect::ALL {
            let src = MockSource::new(MockFc::load(d.as_str(), "speedybeef405v4"));
            let before = src.fc(|fc| fc.snapshot());
            let st = store(&format!("apply-{}", d.as_str()));
            let m = model("cinematic-7in-hd");

            let script = plan(&src, &m).unwrap();
            let mut stages = Vec::new();
            let r = apply(&src, &st, &script, &mut |p| stages.push(p.stage)).unwrap();
            assert!(r.saved && r.reconnected && r.errors.is_empty(), "{r:?}");
            let mut order = stages.clone();
            order.dedup();
            assert_eq!(
                order,
                [
                    ConfigStage::Backup,
                    ConfigStage::Sending,
                    ConfigStage::Saving,
                    ConfigStage::Rebooting,
                    ConfigStage::Reconnected
                ],
                "{d:?}"
            );
            let bk = r.backup.clone().unwrap();
            assert_eq!(bk.board_uid, UID);
            assert!(verify(&src, &m).unwrap().ok, "{d:?}");

            // Restore by id: safety backup, replay, save, reconnect.
            let mut stages = Vec::new();
            let rb = restore(&src, &st, &bk.id, &mut |p| stages.push(p.stage)).unwrap();
            assert!(rb.saved && rb.errors.is_empty(), "{rb:?}");
            assert_eq!(stages.first(), Some(&ConfigStage::Backup));
            assert_eq!(stages.last(), Some(&ConfigStage::Reconnected));
            let after = src.fc(|fc| fc.snapshot());
            assert_eq!(after.ports, before.ports, "{d:?}");
            assert_eq!(after.settings, before.settings, "{d:?}");
            assert_eq!(after.features, before.features, "{d:?}");
            assert_eq!(st.list().unwrap().len(), 2); // apply backup + safety backup

            // config_rollback(Backup) resolves the same backup by id.
            let rb = rollback(&src, &st, &bk, &mut |_| {}).unwrap();
            assert!(rb.saved, "{rb:?}");
            assert_eq!(st.list().unwrap().len(), 3);
            let _ = std::fs::remove_dir_all(st.root());
        }
    }

    #[test]
    fn bad_script_sends_nothing() {
        let src = MockSource::new(MockFc::load("4.5", "speedybeef405v4"));
        let st = store("bad-script");
        let mut script = plan(&src, &model("cinematic-7in-hd")).unwrap();
        src.fc(|fc| fc.log.clear());
        script.commands.push("save".into());
        let e = apply(&src, &st, &script, &mut |_| panic!("no progress")).unwrap_err();
        assert!(matches!(e, AppError::Config(_)), "{e:?}");
        src.fc(|fc| assert!(fc.log.is_empty(), "{:?}", fc.log));
        assert!(st.list().unwrap().is_empty());
    }

    #[test]
    fn rollback_never_opens_the_path_sent_by_the_ui() {
        let src = MockSource::new(MockFc::load("4.5", "speedybeef405v4"));
        let st = store("rb-path");
        let bk = backup(&src, &st).unwrap();
        src.fc(|fc| fc.log.clear());

        // A known id with a foreign path: the stored file is used.
        let mut forged = bk.clone();
        forged.path = "/etc/passwd".into();
        let r = rollback(&src, &st, &forged, &mut |_| {}).unwrap();
        assert!(r.saved, "{r:?}");

        // An id that is not in the store: CONFIG, nothing sent.
        src.fc(|fc| fc.log.clear());
        let mut forged = bk.clone();
        forged.id = "../../etc/passwd".into();
        let e = rollback(&src, &st, &forged, &mut |_| {}).unwrap_err();
        assert!(matches!(e, AppError::Config(_)), "{e:?}");
        src.fc(|fc| assert!(fc.log.is_empty(), "{:?}", fc.log));
        let _ = std::fs::remove_dir_all(st.root());
    }

    #[test]
    fn restore_refuses_another_boards_backup() {
        let st = store("rb-board");
        let other = MockSource::new(MockFc::load("4.5", "speedybeef405v4"));
        let bk = backup(&other, &st).unwrap();

        let mut fc = MockFc::load("4.5", "speedybeef405v4");
        fc.mcu_id = "0033002b3138510b32383733".into();
        let src = MockSource::new(fc);
        let e = restore(&src, &st, &bk.id, &mut |_| {}).unwrap_err();
        assert!(
            matches!(&e, AppError::Config(m) if m.contains("another board")),
            "{e:?}"
        );
        src.fc(|fc| {
            assert!(fc.log.is_empty(), "{:?}", fc.log); // the UID check needs no I/O
            assert_eq!(fc.reboots, 0);
        });
        let _ = std::fs::remove_dir_all(st.root());
    }

    #[test]
    fn restore_rejects_a_tampered_file_before_borrowing_the_port() {
        let src = MockSource::new(MockFc::load("4.5", "speedybeef405v4"));
        let st = store("rb-tamper");
        let bk = backup(&src, &st).unwrap();
        let text = std::fs::read_to_string(&bk.path).unwrap();
        std::fs::write(&bk.path, text.replace("defaults nosave", "defaults")).unwrap();
        src.fc(|fc| fc.log.clear());
        let e = restore(&src, &st, &bk.id, &mut |_| {}).unwrap_err();
        assert!(matches!(e, AppError::Config(_)), "{e:?}");
        src.fc(|fc| assert!(fc.log.is_empty(), "{:?}", fc.log));
        let _ = std::fs::remove_dir_all(st.root());
    }

    #[test]
    fn not_connected_and_port_busy() {
        let empty = MockSource(Mutex::new(None));
        assert!(matches!(read_ports(&empty), Err(AppError::NotConnected)));

        let src = Arc::new(MockSource::new(MockFc::load("4.5", "speedybeef405v4")));
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let s2 = src.clone();
        let t = std::thread::spawn(move || {
            s2.with_link(|_| {
                held_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
        });
        held_rx.recv().unwrap();
        let e = plan(&*src, &model("cinematic-7in-hd")).unwrap_err();
        assert!(
            matches!(
                e,
                AppError::PortBusy {
                    held_by: PortHolder::App,
                    ..
                }
            ),
            "{e:?}"
        );
        release_tx.send(()).unwrap();
        t.join().unwrap().unwrap();
        assert!(plan(&*src, &model("cinematic-7in-hd")).is_ok());
    }

    /// Config's MockFc as the CLI of device's fake FC (which answers MSP), so the real
    /// `ConnectionManager` connects, identifies and serves `with_link` like with a board.
    struct CliOf(MockFc);

    impl CliBackend for CliOf {
        fn enter(&mut self) {
            let _ = self.0.enter_cli();
        }
        fn exec(&mut self, line: &str) -> String {
            self.0
                .cli_exec(line)
                .unwrap_or_else(|e| format!("###ERROR: {e}###"))
        }
        fn reboot(&mut self) {
            let _ = self.0.exit_cli(false);
            let _ = self.0.msp_request(session::MSP_SET_REBOOT, &[0]);
        }
    }

    /// The real connection manager on a fake 4.5 FC: codes come straight from
    /// `with_link`, and the backup folder is the UID the manager read over MSP_UID.
    #[test]
    fn connection_manager_codes_and_uid() {
        let fc = FakeFc::default(); // API 1.46, MSP_UID words 1, 2, 3
        *fc.cli_backend.lock().unwrap() =
            Some(Box::new(CliOf(MockFc::load("4.5", "speedybeef405v4"))));
        let f2 = fc.clone();
        let opener: Opener = Arc::new(move |_| Ok(Box::new(f2.serial()) as BoxLink));
        let lister: Lister = Arc::new(|| {
            vec![PortIdentity {
                path: "COM9".into(),
                vid: Some(0x0483),
                pid: Some(0x5740),
                serial_number: Some("0x8000000".into()),
                ..Default::default()
            }]
        });
        let mgr = Arc::new(
            ConnectionManager::new(opener, lister)
                .with_poll_interval(Duration::from_millis(5))
                .with_msp_timeout(Duration::from_millis(100)),
        );
        assert!(matches!(read_ports(&*mgr), Err(AppError::NotConnected)));
        assert!(matches!(
            verify(&*mgr, &model("cinematic-7in-hd")),
            Err(AppError::NotConnected)
        ));

        let t0 = std::time::Instant::now();
        mgr.connect("COM9").unwrap();
        let st = store("mgr");
        let bk = backup(&*mgr, &st).unwrap();
        assert_eq!(bk.board_uid, "000000010000000200000003");
        assert!(PathBuf::from(&bk.path).starts_with(st.root().join(&bk.board_uid)));
        assert!(!read_ports(&*mgr).unwrap().is_empty());
        assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());

        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let m2 = mgr.clone();
        let t = std::thread::spawn(move || {
            m2.with_link(|_| {
                held_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
        });
        held_rx.recv().unwrap();
        let e = backup(&*mgr, &st).unwrap_err();
        assert!(
            matches!(&e, AppError::PortBusy { port, held_by: PortHolder::App } if port == "COM9"),
            "{e:?}"
        );
        release_tx.send(()).unwrap();
        t.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(st.root());
    }
}
