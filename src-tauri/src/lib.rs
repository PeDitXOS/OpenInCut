//! Backend Tauri: expone el ProjectStore de ue-core como comandos IPC.
//! El frontend consulta el estado tras cada mutación (v0); los eventos
//! `state.patch` llegarán cuando el volumen de datos lo justifique.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use ue_audio::items::{collect_specs, load_items};
use ue_audio::player::Player;
use ue_core::model::{AudioProps, Clip, Id, MediaKind, Project, TrackKind, Transform2D};
use ue_core::ops::InsertMode;
use ue_core::{ProjectStore, TimeUs};

pub struct AppState {
    pub store: Mutex<ProjectStore>,
    pub path: Mutex<Option<PathBuf>>,
    pub cache_dir: Mutex<Option<PathBuf>>,
    pub player: Mutex<Option<Player>>,
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
fn playback_play(state: State<AppState>, from_us: TimeUs) -> Res<()> {
    sync_player(&state)?;
    let guard = state.player.lock().unwrap();
    guard.as_ref().unwrap().play(from_us);
    Ok(())
}

#[tauri::command]
fn playback_pause(state: State<AppState>) -> Res<TimeUs> {
    let guard = state.player.lock().unwrap();
    match guard.as_ref() {
        Some(p) => Ok(p.pause()),
        None => Err("sin reproductor".into()),
    }
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
    let bytes = ue_media::frame::render_frame(&project, seq_id, t_us, max_width, &base_dir)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    Ok(tauri::ipc::Response::new(bytes))
}

/// Exporta la secuencia activa a MP4 (bloqueante en un hilo aparte).
#[tauri::command]
async fn export_video(
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
    let out = PathBuf::from(&path);
    let settings = ue_export::ExportSettings { max_height, ..Default::default() };
    tauri::async_runtime::spawn_blocking(move || {
        ue_export::export_sequence(&project, seq_id, &base_dir, &out, &settings)
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
            export_video,
            playback_play,
            playback_pause,
            playback_seek,
            playback_position,
            save_project,
            open_project,
            new_project,
        ])
        .run(tauri::generate_context!())
        .expect("error al arrancar UberEditor");
}
