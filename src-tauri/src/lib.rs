//! Backend Tauri: expone el ProjectStore de ue-core como comandos IPC.
//! El frontend consulta el estado tras cada mutación (v0); los eventos
//! `state.patch` llegarán cuando el volumen de datos lo justifique.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use ue_audio::items::{collect_specs, load_items};
use ue_audio::player::Player;
use ue_core::model::{AudioProps, Clip, Id, MediaKind, Project, TrackKind, Transform2D};
use ue_core::ops::InsertMode;
use ue_core::{ProjectStore, TimeUs};
use ue_media::stream::MjpegSession;

pub struct AppState {
    pub store: Mutex<ProjectStore>,
    pub path: Mutex<Option<PathBuf>>,
    pub cache_dir: Mutex<Option<PathBuf>>,
    pub player: Mutex<Option<Player>>,
    pub frames: Mutex<Option<FrameService>>,
    pub export_cancel: Arc<AtomicBool>,
    /// Registro efectivo (core + packs de usuario) y packs de usuario crudos.
    pub registry: Mutex<Arc<Vec<ue_render::EffectDef>>>,
    pub user_packs: Mutex<Vec<ue_render::EffectDef>>,
    pub effects_dir: Mutex<Option<PathBuf>>,
}

/// Recarga los packs de usuario desde disco y reconstruye el registro.
/// Devuelve los errores de manifests inválidos (no rompen nada).
fn reload_packs(state: &AppState) -> Vec<String> {
    let dir = state.effects_dir.lock().unwrap().clone();
    let (user, errors) = match dir {
        Some(d) => ue_render::load_packs_from_dir(&d),
        None => (vec![], vec![]),
    };
    let merged = ue_render::merge_registries(ue_render::core_registry(), user.clone());
    *state.user_packs.lock().unwrap() = user;
    *state.registry.lock().unwrap() = Arc::new(merged);
    errors
}

/// Servicio de frames de reproducción: un hilo sigue al reloj de audio con una
/// sesión MJPEG persistente y publica el último frame decodificado.
pub struct FrameService {
    pub latest: Arc<Mutex<Vec<u8>>>,
    pub running: Arc<AtomicBool>,
}

const PLAYBACK_FPS: u32 = 24;
const PLAYBACK_MAX_W: u32 = 960;

fn frame_service_loop(app: tauri::AppHandle, latest: Arc<Mutex<Vec<u8>>>, running: Arc<AtomicBool>) {
    let mut session: Option<MjpegSession> = None;
    let mut session_vf: Option<String> = None;
    while running.load(Ordering::SeqCst) {
        let state = app.state::<AppState>();
        let (t, playing) = {
            let guard = state.player.lock().unwrap();
            match guard.as_ref() {
                Some(p) => (p.position_us(), p.is_playing()),
                None => (0, false),
            }
        };
        if !playing {
            break;
        }
        let resolved = {
            let store = state.store.lock().unwrap();
            ue_media::frame::resolve_top_video(
                &store.project,
                store.project.active_sequence,
                t,
            )
        };
        let Some(r) = resolved else {
            latest.lock().unwrap().clear();
            session = None;
            std::thread::sleep(Duration::from_millis(40));
            continue;
        };
        let path = PathBuf::from(&r.asset_path);
        let src_t = r.src_t_us;
        let reg = state.registry.lock().unwrap().clone();
        let vf = ue_render::clip_vf(&reg, &r.effects, &r.transform);

        // ¿sirve la sesión actual? (mismo archivo, misma cadena de efectos,
        // posición alcanzable hacia delante)
        let reusable = session.as_ref().is_some_and(|s| {
            s.asset_path == path
                && session_vf == vf
                && src_t >= s.next_src_us() - 1_000_000 / PLAYBACK_FPS as i64
                && src_t <= s.next_src_us() + 1_500_000
        });
        if !reusable {
            session =
                MjpegSession::open(&path, src_t, PLAYBACK_MAX_W, PLAYBACK_FPS, vf.as_deref()).ok();
            session_vf = vf;
        }
        if let Some(s) = session.as_mut() {
            let mut newest: Option<Vec<u8>> = None;
            let mut dead = false;
            while s.next_src_us() <= src_t {
                match s.next_frame() {
                    Ok(Some(f)) => newest = Some(f),
                    _ => {
                        dead = true;
                        break;
                    }
                }
            }
            if dead {
                session = None;
            }
            if let Some(f) = newest {
                *latest.lock().unwrap() = f;
            }
        }
        std::thread::sleep(Duration::from_millis(1000 / PLAYBACK_FPS as u64 / 2));
    }
    running.store(false, Ordering::SeqCst);
}

fn start_frame_service(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let mut guard = state.frames.lock().unwrap();
    if let Some(fs) = guard.as_ref() {
        if fs.running.load(Ordering::SeqCst) {
            return; // ya corre
        }
    }
    let latest = Arc::new(Mutex::new(Vec::new()));
    let running = Arc::new(AtomicBool::new(true));
    *guard = Some(FrameService { latest: latest.clone(), running: running.clone() });
    let app2 = app.clone();
    std::thread::spawn(move || frame_service_loop(app2, latest, running));
}

fn stop_frame_service(state: &AppState) {
    if let Some(fs) = state.frames.lock().unwrap().as_ref() {
        fs.running.store(false, Ordering::SeqCst);
    }
}

/// Ruta del WAV conformado de un asset en la caché de la app.
fn conform_target(cache_dir: &Path, content_hash: &str) -> PathBuf {
    cache_dir.join(content_hash.replace(':', "-")).join("audio.wav")
}

/// Sincroniza los items del mezclador con el estado actual (si cambió).
/// Orden de locks SIEMPRE: store → player.
fn sync_player(state: &AppState) -> Result<(), String> {
    let store = state.store.lock().unwrap();
    let mut player_guard = state.player.lock().unwrap();
    if player_guard.is_none() {
        *player_guard = Some(Player::new().map_err(|e| e.to_string())?);
    }
    let player = player_guard.as_ref().unwrap();
    // versión+1 para distinguir del 0 inicial del player
    if player.items_version() != store.version + 1 {
        let specs = collect_specs(&store.project, store.project.active_sequence);
        let (items, _skipped) =
            load_items(&store.project, &specs, |a| a.audio_conform.as_ref().map(PathBuf::from));
        player.set_items(items, store.version + 1);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct StateSnapshot {
    pub project: Project,
    pub version: u64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_labels: Vec<String>,
}

fn snapshot(store: &ProjectStore) -> StateSnapshot {
    StateSnapshot {
        project: store.project.clone(),
        version: store.version,
        dirty: store.dirty,
        can_undo: store.can_undo(),
        can_redo: store.can_redo(),
        undo_labels: store.undo_labels().iter().map(|s| s.to_string()).collect(),
    }
}

fn parse_id(s: &str) -> Result<Id, String> {
    s.parse::<Id>().map_err(|e| format!("id inválido '{s}': {e}"))
}

type Res<T> = Result<T, String>;

#[tauri::command]
fn get_state(state: State<AppState>) -> Res<StateSnapshot> {
    Ok(snapshot(&state.store.lock().unwrap()))
}

#[tauri::command]
fn split_clip(state: State<AppState>, clip_id: String, t_us: TimeUs) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    store.split_clip(parse_id(&clip_id)?, t_us).map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn delete_clips(state: State<AppState>, ids: Vec<String>, ripple: bool) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    let ids: Result<Vec<Id>, String> = ids.iter().map(|s| parse_id(s)).collect();
    store.delete_clips(&ids?, ripple).map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn move_clip(
    state: State<AppState>,
    clip_id: String,
    to_track: String,
    to_start_us: TimeUs,
    overwrite: bool,
) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    let mode = if overwrite { InsertMode::Overwrite } else { InsertMode::Strict };
    store
        .move_clip(parse_id(&clip_id)?, parse_id(&to_track)?, to_start_us, mode)
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn trim_clip(
    state: State<AppState>,
    clip_id: String,
    left: bool,
    new_edge_us: TimeUs,
) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    store.trim_clip(parse_id(&clip_id)?, left, new_edge_us).map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn cut_ranges(
    state: State<AppState>,
    sequence_id: String,
    ranges: Vec<(TimeUs, TimeUs)>,
    ripple: bool,
) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    store
        .cut_ranges(parse_id(&sequence_id)?, &ranges, ripple)
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn undo(state: State<AppState>) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    store.undo().map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn redo(state: State<AppState>) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    store.redo().map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn set_clip_audio(state: State<AppState>, clip_id: String, audio: AudioProps) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    let id = parse_id(&clip_id)?;
    store
        .dispatch(
            "Editar audio",
            vec![ue_core::Action::SetClipAudio { clip_id: id, audio }],
        )
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn set_clip_transform(
    state: State<AppState>,
    clip_id: String,
    transform: Transform2D,
) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    let id = parse_id(&clip_id)?;
    store
        .dispatch(
            "Editar transformación",
            vec![ue_core::Action::SetClipTransform { clip_id: id, transform }],
        )
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

/// Importa archivos al pool (probe + hash). No entra al historial (PLAN §6.10).
/// El conformado de audio se lanza en segundo plano; al terminar se emite
/// `state-changed` para que la UI refresque.
#[tauri::command]
fn import_media(
    app: tauri::AppHandle,
    state: State<AppState>,
    paths: Vec<String>,
) -> Res<StateSnapshot> {
    let cache_dir = state.cache_dir.lock().unwrap().clone();
    let mut store = state.store.lock().unwrap();
    let mut errors: Vec<String> = vec![];
    let mut imported = 0usize;
    for p in &paths {
        match ue_media::import_file(Path::new(p)) {
            Ok(asset) => {
                // re-import del mismo contenido → no duplicar
                if !store.project.assets.iter().any(|a| a.content_hash == asset.content_hash) {
                    if asset.probe.audio_channels > 0 {
                        if let Some(cache) = &cache_dir {
                            spawn_conform_job(&app, &asset, cache);
                        }
                    }
                    store.project.assets.push(asset);
                }
                imported += 1;
            }
            Err(e) => errors.push(format!("{p}: {e}")),
        }
    }
    if imported > 0 {
        store.version += 1;
        store.dirty = true;
    }
    if imported == 0 && !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(snapshot(&store))
}

fn spawn_conform_job(app: &tauri::AppHandle, asset: &ue_core::model::MediaAsset, cache: &Path) {
    let app = app.clone();
    let asset_id = asset.id;
    let src = PathBuf::from(&asset.path);
    let out = conform_target(cache, &asset.content_hash);
    std::thread::spawn(move || {
        match ue_media::conform_audio(&src, &out) {
            Ok(()) => {
                let state = app.state::<AppState>();
                {
                    let mut store = state.store.lock().unwrap();
                    if let Some(a) = store.project.assets.iter_mut().find(|a| a.id == asset_id) {
                        a.audio_conform = Some(out.to_string_lossy().into_owned());
                    }
                    store.version += 1;
                }
                let _ = app.emit("state-changed", ());
            }
            Err(e) => eprintln!("[conform] {src:?}: {e}"),
        }
    });
}

// ---- transporte (el audio es el reloj maestro) ----

#[tauri::command]
fn playback_play(app: tauri::AppHandle, state: State<AppState>, from_us: TimeUs) -> Res<()> {
    sync_player(&state)?;
    {
        let guard = state.player.lock().unwrap();
        guard.as_ref().unwrap().play(from_us);
    }
    start_frame_service(&app);
    Ok(())
}

#[tauri::command]
fn playback_pause(state: State<AppState>) -> Res<TimeUs> {
    stop_frame_service(&state);
    let guard = state.player.lock().unwrap();
    match guard.as_ref() {
        Some(p) => Ok(p.pause()),
        None => Err("sin reproductor".into()),
    }
}

/// Último frame del stream de reproducción (vacío = sin señal todavía).
#[tauri::command]
fn playback_frame(state: State<AppState>) -> Res<tauri::ipc::Response> {
    let bytes = match state.frames.lock().unwrap().as_ref() {
        Some(fs) => fs.latest.lock().unwrap().clone(),
        None => vec![],
    };
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
fn playback_seek(state: State<AppState>, t_us: TimeUs) -> Res<()> {
    if let Some(p) = state.player.lock().unwrap().as_ref() {
        p.seek(t_us);
    }
    Ok(())
}

/// (posición µs, reproduciendo). También re-sincroniza los items si el
/// proyecto cambió durante la reproducción (editar mientras suena).
#[tauri::command]
fn playback_position(state: State<AppState>) -> Res<(TimeUs, bool)> {
    let _ = sync_player(&state); // barato si no cambió la versión
    let guard = state.player.lock().unwrap();
    match guard.as_ref() {
        Some(p) => Ok((p.position_us(), p.is_playing())),
        None => Err("sin reproductor".into()),
    }
}

/// Añade un clip del asset a la primera pista compatible: en `at_us` si cabe,
/// si no al final de la pista.
#[tauri::command]
fn add_clip(state: State<AppState>, asset_id: String, at_us: TimeUs) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    let asset_id = parse_id(&asset_id)?;
    let asset = store
        .project
        .asset(asset_id)
        .ok_or_else(|| format!("asset {asset_id} no existe"))?
        .clone();
    let duration = ue_media::default_clip_duration(&asset);
    if duration <= 0 {
        return Err("el archivo no tiene duración utilizable".into());
    }
    let want_kind = if asset.kind == MediaKind::Audio { TrackKind::Audio } else { TrackKind::Video };
    let seq_id = store.project.active_sequence;
    let seq = store.project.sequence(seq_id).ok_or("secuencia activa no existe")?;
    let track = seq
        .tracks
        .iter()
        .find(|t| t.kind == want_kind && !t.locked)
        .ok_or("no hay pista compatible desbloqueada")?;
    let track_id = track.id;
    let at = at_us.max(0);
    let fits = !track.collides(at, duration, None);
    let start = if fits {
        at
    } else {
        track.clips.iter().map(|c| c.end()).max().unwrap_or(0)
    };
    let clip = Clip::new_media(asset.id, 0, duration, start);
    store
        .insert_clip(track_id, clip, InsertMode::Strict)
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

/// Frame real JPEG del tiempo dado (bytes crudos; vacío = sin señal).
#[tauri::command]
fn render_frame(
    state: State<AppState>,
    t_us: TimeUs,
    max_width: u32,
) -> Res<tauri::ipc::Response> {
    let (project, seq_id, base_dir) = {
        let store = state.store.lock().unwrap();
        let base = state
            .path
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        (store.project.clone(), store.project.active_sequence, base)
    }; // soltar el lock antes de invocar ffmpeg
    let reg = state.registry.lock().unwrap().clone();
    let vf = ue_media::frame::resolve_top_video(&project, seq_id, t_us)
        .and_then(|r| ue_render::clip_vf(&reg, &r.effects, &r.transform));
    let bytes =
        ue_media::frame::render_frame(&project, seq_id, t_us, max_width, &base_dir, vf.as_deref())
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
    Ok(tauri::ipc::Response::new(bytes))
}

/// Catálogo de efectos disponibles (para la UI y MCP).
#[tauri::command]
fn get_effects_catalog(state: State<AppState>) -> serde_json::Value {
    ue_render::catalog_json(&state.registry.lock().unwrap())
}

/// Recarga los packs de usuario desde disco (carpeta effects/ de la config).
#[tauri::command]
fn reload_effect_packs(state: State<AppState>) -> Res<serde_json::Value> {
    let errors = reload_packs(&state);
    Ok(serde_json::json!({
        "catalog": ue_render::catalog_json(&state.registry.lock().unwrap()),
        "errors": errors,
        "dir": state.effects_dir.lock().unwrap().as_ref().map(|d| d.display().to_string()),
    }))
}

#[tauri::command]
fn set_clip_effects(
    state: State<AppState>,
    clip_id: String,
    effects: Vec<ue_core::model::EffectInstance>,
) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    let id = parse_id(&clip_id)?;
    store
        .dispatch(
            "Editar efectos",
            vec![ue_core::Action::SetClipEffects { clip_id: id, effects }],
        )
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn set_clip_transition(
    state: State<AppState>,
    clip_id: String,
    transition: Option<ue_core::model::TransitionRef>,
) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    let id = parse_id(&clip_id)?;
    store
        .dispatch(
            "Editar transición",
            vec![ue_core::Action::SetClipTransition { clip_id: id, transition }],
        )
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&store))
}

#[tauri::command]
fn cancel_export(state: State<AppState>) -> Res<()> {
    state.export_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

/// Exporta la secuencia activa a MP4 (bloqueante en un hilo aparte).
/// Emite eventos `export-progress` (0..1); `cancel_export` la aborta.
#[tauri::command]
async fn export_video(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    max_height: Option<u32>,
) -> Res<String> {
    let (project, seq_id, base_dir) = {
        let store = state.store.lock().unwrap();
        let base = state
            .path
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        (store.project.clone(), store.project.active_sequence, base)
    };
    let cancel = state.export_cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    let out = PathBuf::from(&path);
    let extra_packs = state.user_packs.lock().unwrap().clone();
    let settings = ue_export::ExportSettings { max_height, extra_packs, ..Default::default() };
    tauri::async_runtime::spawn_blocking(move || {
        ue_export::export_sequence_with_progress(
            &project,
            seq_id,
            &base_dir,
            &out,
            &settings,
            |p| {
                let _ = app.emit("export-progress", p);
            },
            &cancel,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(path)
}

#[tauri::command]
fn save_project(state: State<AppState>, path: Option<String>) -> Res<String> {
    let mut store = state.store.lock().unwrap();
    let mut stored_path = state.path.lock().unwrap();
    let target = match path.map(PathBuf::from).or_else(|| stored_path.clone()) {
        Some(p) => p,
        None => return Err("no hay ruta de guardado; pasa una ruta".into()),
    };
    let json = store.project.to_json().map_err(|e| e.to_string())?;
    // escritura atómica: tmp + rename
    let tmp = target.with_extension("uep.tmp");
    std::fs::write(&tmp, &json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    store.dirty = false;
    *stored_path = Some(target.clone());
    Ok(target.display().to_string())
}

#[tauri::command]
fn open_project(state: State<AppState>, path: String) -> Res<StateSnapshot> {
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let project = Project::from_json(&json).map_err(|e| e.to_string())?;
    let issues = ue_core::validate::validate(&project);
    if !issues.is_empty() {
        return Err(format!("proyecto inválido: {}", issues.join("; ")));
    }
    let mut store = state.store.lock().unwrap();
    *store = ProjectStore::new(project);
    *state.path.lock().unwrap() = Some(PathBuf::from(path));
    Ok(snapshot(&store))
}

#[tauri::command]
fn new_project(state: State<AppState>, name: String) -> Res<StateSnapshot> {
    let mut store = state.store.lock().unwrap();
    *store = ProjectStore::new(Project::new(&name));
    *state.path.lock().unwrap() = None;
    Ok(snapshot(&store))
}

pub fn run() {
    let state = AppState {
        store: Mutex::new(ProjectStore::new(Project::new("Proyecto sin título"))),
        path: Mutex::new(None),
        cache_dir: Mutex::new(None),
        player: Mutex::new(None),
        frames: Mutex::new(None),
        export_cancel: Arc::new(AtomicBool::new(false)),
        registry: Mutex::new(Arc::new(ue_render::core_registry())),
        user_packs: Mutex::new(vec![]),
        effects_dir: Mutex::new(None),
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(|app| {
            let state = app.state::<AppState>();
            if let Ok(dir) = app.path().app_cache_dir() {
                let _ = std::fs::create_dir_all(&dir);
                *state.cache_dir.lock().unwrap() = Some(dir);
            }
            if let Ok(dir) = app.path().app_config_dir() {
                let effects = dir.join("effects");
                let _ = std::fs::create_dir_all(&effects);
                *state.effects_dir.lock().unwrap() = Some(effects);
                let errors = reload_packs(&state);
                for e in errors {
                    eprintln!("[packs] manifest inválido: {e}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            split_clip,
            delete_clips,
            move_clip,
            trim_clip,
            cut_ranges,
            undo,
            redo,
            set_clip_audio,
            set_clip_transform,
            import_media,
            add_clip,
            render_frame,
            get_effects_catalog,
            reload_effect_packs,
            set_clip_effects,
            set_clip_transition,
            export_video,
            cancel_export,
            playback_play,
            playback_pause,
            playback_seek,
            playback_position,
            playback_frame,
            save_project,
            open_project,
            new_project,
        ])
        .run(tauri::generate_context!())
        .expect("error al arrancar UberEditor");
}
