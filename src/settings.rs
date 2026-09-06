use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow, WindowMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

pub const RESOLUTIONS: [(u32, u32); 5] = [
    (1280, 720),
    (1600, 900),
    (1920, 1080),
    (2560, 1440),
    (3840, 2160),
];

/// Selectable FPS caps in the settings menu.  Index 0 = UNLIMITED;
/// remaining entries are sorted ascending so cycling left/right walks
/// monotonically through the values.  `fps_limiter` enforces these.
pub const FPS_CAPS: [Option<u32>; 10] = [
    None,
    Some(30),
    Some(60),
    Some(120),
    Some(144),
    Some(165),
    Some(200),
    Some(300),
    Some(400),
    Some(500),
];

pub const QUALITY_LABELS: [&str; 4] = ["LOW", "MEDIUM", "HIGH", "ULTRA"];

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowModeChoice {
    Windowed,
    Borderless,
    Fullscreen,
}

impl WindowModeChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Windowed => "WINDOWED",
            Self::Borderless => "BORDERLESS",
            Self::Fullscreen => "FULLSCREEN",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Windowed => Self::Borderless,
            Self::Borderless => Self::Fullscreen,
            Self::Fullscreen => Self::Windowed,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Self::Windowed => Self::Fullscreen,
            Self::Borderless => Self::Windowed,
            Self::Fullscreen => Self::Borderless,
        }
    }
}

/// Default master volume — also the fallback when loading a settings file
/// written before the `volume` field existed (`serde(default)`).
fn default_volume() -> f32 {
    0.8
}

#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct GraphicsSettings {
    pub resolution_idx: usize,
    pub window_mode: WindowModeChoice,
    pub vsync: bool,
    pub fps_cap_idx: usize,
    pub quality_idx: usize,
    pub show_fps: bool,
    /// Master volume 0.0–1.0, applied to every sound (the only setting exposed
    /// on mobile).  `serde(default)` keeps older on-disk settings loadable.
    #[serde(default = "default_volume")]
    pub volume: f32,
}

#[derive(Resource, Default)]
pub struct SettingsLoadedFromDisk(pub bool);

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            resolution_idx: 0,
            window_mode: WindowModeChoice::Borderless,
            vsync: true,
            fps_cap_idx: 0,
            quality_idx: 2,
            show_fps: false,
            volume: default_volume(),
        }
    }
}

impl GraphicsSettings {
    /// Clamp everything read back from disk / localStorage into range.  The
    /// option lists are indexed directly, so a hand-edited or stale file
    /// (an index from a longer list in an older build) would otherwise panic
    /// on the first frame, every launch.
    pub fn sanitized(mut self) -> Self {
        self.resolution_idx = self.resolution_idx.min(RESOLUTIONS.len() - 1);
        self.fps_cap_idx = self.fps_cap_idx.min(FPS_CAPS.len() - 1);
        self.quality_idx = self.quality_idx.min(QUALITY_LABELS.len() - 1);
        self.volume = if self.volume.is_finite() { self.volume.clamp(0.0, 1.0) } else { default_volume() };
        self
    }

    pub fn resolution_label(&self) -> String {
        let (w, h) = RESOLUTIONS[self.resolution_idx];
        format!("{w} x {h}")
    }
    pub fn fps_cap_label(&self) -> String {
        match FPS_CAPS[self.fps_cap_idx] {
            None => "UNLIMITED".to_string(),
            Some(v) => format!("{v} FPS"),
        }
    }
    pub fn vsync_label(&self) -> &'static str {
        if self.vsync {
            "ON"
        } else {
            "OFF"
        }
    }
    pub fn window_mode_label(&self) -> &'static str {
        self.window_mode.label()
    }

    pub fn cycle_resolution(&mut self, forward: bool) {
        let len = RESOLUTIONS.len();
        self.resolution_idx = if forward {
            (self.resolution_idx + 1) % len
        } else {
            (self.resolution_idx + len - 1) % len
        };
    }

    pub fn cycle_fps_cap(&mut self, forward: bool) {
        let len = FPS_CAPS.len();
        self.fps_cap_idx = if forward {
            (self.fps_cap_idx + 1) % len
        } else {
            (self.fps_cap_idx + len - 1) % len
        };
    }

    pub fn cycle_window_mode(&mut self, forward: bool) {
        self.window_mode = if forward {
            self.window_mode.next()
        } else {
            self.window_mode.prev()
        };
    }

    pub fn toggle_vsync(&mut self) {
        self.vsync = !self.vsync;
    }

    pub fn cycle_quality(&mut self, forward: bool) {
        let len = QUALITY_LABELS.len();
        self.quality_idx = if forward {
            (self.quality_idx + 1) % len
        } else {
            (self.quality_idx + len - 1) % len
        };
    }

    pub fn quality_label(&self) -> &'static str {
        QUALITY_LABELS[self.quality_idx]
    }

    pub fn toggle_show_fps(&mut self) {
        self.show_fps = !self.show_fps;
    }

    pub fn volume_label(&self) -> String {
        format!("{}%", (self.volume * 100.0).round() as i32)
    }

    /// Step the master volume by 10%, clamped to 0–100% and snapped to the grid
    /// so floating-point drift can't leave it at e.g. 79%.
    pub fn cycle_volume(&mut self, forward: bool) {
        let steps = (self.volume * 10.0).round() + if forward { 1.0 } else { -1.0 };
        self.volume = (steps.clamp(0.0, 10.0)) / 10.0;
    }

    pub fn show_fps_label(&self) -> &'static str {
        if self.show_fps { "ON" } else { "OFF" }
    }
}

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let (settings, loaded) = match load_settings() {
            Some(s) => (s, true),
            None => (GraphicsSettings::default(), false),
        };
        app.insert_resource(settings)
            .insert_resource(SettingsLoadedFromDisk(loaded))
            .add_systems(
                Update,
                (detect_initial_resolution, apply_graphics_settings, save_settings_on_change),
            );
        // The browser paces frames with requestAnimationFrame and has neither
        // `Instant` nor `thread::sleep`, so the limiter is native-only.
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(Last, fps_limiter);
    }
}

/// Per-platform writable base directory for persistent data, shared by
/// settings and achievements (see `achievements::save_dir`).  `None` means
/// no platform dir could be resolved; callers fall back to the cwd.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(target_os = "linux")]
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        });
    #[cfg(target_os = "windows")]
    let base = std::env::var("APPDATA").ok().map(PathBuf::from);
    // The browser has no filesystem: the path only names the localStorage key.
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let base: Option<PathBuf> = None;

    base
}

fn settings_path() -> PathBuf {
    data_dir()
        .map(|b| b.join("zombiegame2"))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("settings.json")
}

fn load_settings() -> Option<GraphicsSettings> {
    let path = settings_path();
    let data = match crate::storage::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warn!(
                "Failed to read graphics settings at {}: {}. Using defaults.",
                path.display(),
                e
            );
            return None;
        }
    };
    match serde_json::from_str::<GraphicsSettings>(&data) {
        Ok(s) => Some(s.sanitized()),
        Err(e) => {
            let mut bak = path.clone();
            bak.set_extension("json.bak");
            match crate::storage::write(&bak, &data) {
                Ok(()) => warn!(
                    "Graphics settings at {} are corrupted ({}). Backed up to {}, using defaults.",
                    path.display(),
                    e,
                    bak.display()
                ),
                Err(be) => warn!(
                    "Graphics settings at {} are corrupted ({}); backup to {} failed ({}). Using defaults.",
                    path.display(),
                    e,
                    bak.display(),
                    be
                ),
            }
            None
        }
    }
}

fn save_settings(settings: &GraphicsSettings) {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = crate::storage::create_dir_all(parent) {
            warn!("Failed to create settings dir {}: {}", parent.display(), e);
        }
    }
    if let Ok(data) = serde_json::to_string_pretty(settings) {
        if let Err(e) = crate::storage::write(&path, &data) {
            warn!("Failed to save settings to {}: {}", path.display(), e);
        }
    }
}

fn save_settings_on_change(
    settings: Res<GraphicsSettings>,
    mut skip_first: Local<bool>,
) {
    if !settings.is_changed() {
        return;
    }
    if !*skip_first {
        *skip_first = true;
        return;
    }
    save_settings(&settings);
}

fn detect_initial_resolution(
    mut settings: ResMut<GraphicsSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    loaded: Res<SettingsLoadedFromDisk>,
    mut ran: Local<bool>,
) {
    if *ran {
        return;
    }
    *ran = true;
    if loaded.0 {
        return;
    }
    let Ok(window) = windows.get_single() else {
        return;
    };
    let w = window.physical_width();
    let h = window.physical_height();
    let mut best = 0;
    let mut best_diff = u32::MAX;
    for (i, &(rw, rh)) in RESOLUTIONS.iter().enumerate() {
        let diff = w.abs_diff(rw) + h.abs_diff(rh);
        if diff < best_diff {
            best_diff = diff;
            best = i;
        }
    }
    settings.resolution_idx = best;
}

fn apply_graphics_settings(
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !settings.is_changed() {
        return;
    }
    // On phones the window is OS-managed fullscreen and the only exposed setting
    // is volume, so never poke the window here — a volume change must not try to
    // restyle resolution / window mode / present mode.
    // Same in the browser: the "window" is a canvas sized by the page and
    // fullscreen needs a user gesture, so window settings are a no-op there.
    if crate::mobile_profile() || cfg!(target_arch = "wasm32") {
        return;
    }
    let Ok(mut window) = windows.get_single_mut() else {
        return;
    };
    let (w, h) = RESOLUTIONS[settings.resolution_idx];
    window.mode = match settings.window_mode {
        WindowModeChoice::Windowed => WindowMode::Windowed,
        WindowModeChoice::Borderless => WindowMode::BorderlessFullscreen,
        WindowModeChoice::Fullscreen => WindowMode::Fullscreen,
    };
    if matches!(settings.window_mode, WindowModeChoice::Windowed) {
        window.resolution.set(w as f32, h as f32);
    }
    window.present_mode = if settings.vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
}

/// Frame pacing without `thread::sleep` (which on macOS/Windows has 1-15 ms
/// granularity and clashes with the swapchain).  Strategy:
/// 1. If we're more than 2 ms early, do a short *coarse* sleep that
///    intentionally undershoots the deadline (so we never overshoot).
/// 2. Spin-yield the remaining sub-millisecond gap with `std::hint::spin_loop`
///    + `thread::yield_now` for tight pacing without burning a core.
///
/// Result: stable cap at the requested FPS without the visible stutter that
/// pure `thread::sleep(target - elapsed)` introduces under VSync coupling.
#[cfg(not(target_arch = "wasm32"))]
fn fps_limiter(settings: Res<GraphicsSettings>, mut last: Local<Option<Instant>>) {
    let Some(cap) = FPS_CAPS[settings.fps_cap_idx] else {
        *last = None;
        return;
    };
    let target = Duration::from_secs_f64(1.0 / cap as f64);
    let prev = match *last {
        Some(p) => p,
        None => {
            *last = Some(Instant::now());
            return;
        }
    };
    let deadline = prev + target;

    // Coarse sleep: aim to wake ~1.5 ms before deadline so we never overshoot.
    const SLEEP_MARGIN: Duration = Duration::from_micros(1500);
    let now = Instant::now();
    let safe_wake = deadline.checked_sub(SLEEP_MARGIN);
    if let Some(wake_at) = safe_wake {
        if let Some(sleep_for) = wake_at.checked_duration_since(now) {
            if sleep_for > Duration::ZERO {
                std::thread::sleep(sleep_for);
            }
        }
    }

    // Fine spin-yield to the deadline.  yield_now hands the core back so we
    // don't peg a CPU at 100 %; spin_loop is the SMT-friendly nop.
    while Instant::now() < deadline {
        std::hint::spin_loop();
        std::thread::yield_now();
    }
    *last = Some(deadline);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden fixture: a settings.json written before the `volume` field
    /// existed.  Players' on-disk files are never migrated, only
    /// deserialized — this exact shape must keep loading forever, or
    /// `load_settings` treats the file as corrupt and resets everything.
    const PRE_VOLUME_FILE: &str = r#"{
        "resolution_idx": 2,
        "window_mode": "Fullscreen",
        "vsync": false,
        "fps_cap_idx": 3,
        "quality_idx": 1,
        "show_fps": true
    }"#;

    #[test]
    fn pre_volume_settings_file_loads_with_default_volume() {
        let s: GraphicsSettings =
            serde_json::from_str(PRE_VOLUME_FILE).expect("old settings file must load");
        assert_eq!(s.volume, 0.8, "missing volume must fall back to default_volume()");
        // Every present field survives untouched.
        assert_eq!(s.resolution_idx, 2);
        assert!(s.window_mode == WindowModeChoice::Fullscreen);
        assert!(!s.vsync);
        assert_eq!(s.fps_cap_idx, 3);
        assert_eq!(s.quality_idx, 1);
        assert!(s.show_fps);
    }

    #[test]
    fn settings_roundtrip_preserves_every_field() {
        let orig = GraphicsSettings {
            resolution_idx: 4,
            window_mode: WindowModeChoice::Windowed,
            vsync: false,
            fps_cap_idx: 9,
            quality_idx: 3,
            show_fps: true,
            volume: 0.3,
        };
        let json = serde_json::to_string(&orig).unwrap();
        let back: GraphicsSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.resolution_idx, orig.resolution_idx);
        assert!(back.window_mode == orig.window_mode);
        assert_eq!(back.vsync, orig.vsync);
        assert_eq!(back.fps_cap_idx, orig.fps_cap_idx);
        assert_eq!(back.quality_idx, orig.quality_idx);
        assert_eq!(back.show_fps, orig.show_fps);
        assert_eq!(back.volume, orig.volume);
    }

    /// Forcing function for the back-compat contract: today every field
    /// except `volume` is REQUIRED, so an empty object fails.  If you add
    /// a new field WITHOUT `#[serde(default)]`, PRE_VOLUME_FILE above
    /// starts failing too — which is exactly what would happen to every
    /// existing player's settings.json on upgrade (load_settings backs the
    /// file up and resets all settings).  Give new fields a serde default.
    #[test]
    fn missing_required_field_is_treated_as_corruption() {
        assert!(serde_json::from_str::<GraphicsSettings>("{}").is_err());
        // volume is currently the only optional field:
        assert!(serde_json::from_str::<GraphicsSettings>(PRE_VOLUME_FILE).is_ok());
    }
}
