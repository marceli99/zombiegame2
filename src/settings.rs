use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow, WindowMode};
use std::time::{Duration, Instant};

pub const RESOLUTIONS: [(u32, u32); 5] = [
    (1280, 720),
    (1600, 900),
    (1920, 1080),
    (2560, 1440),
    (3840, 2160),
];

pub const FPS_CAPS: [Option<u32>; 5] = [
    None,
    Some(60),
    Some(120),
    Some(144),
    Some(240),
];

pub const QUALITY_LABELS: [&str; 4] = ["LOW", "MEDIUM", "HIGH", "ULTRA"];

pub struct QualityPreset {
    pub dirt: usize,
    pub leaves: usize,
    pub twigs: usize,
    pub grass: usize,
    pub bushes: usize,
    pub trees: usize,
    pub props: usize,
    pub rain: usize,
}

pub const QUALITY_PRESETS: [QualityPreset; 4] = [
    QualityPreset { dirt: 20, leaves: 60, twigs: 20, grass: 140, bushes: 30, trees: 100, props: 70, rain: 30 },
    QualityPreset { dirt: 30, leaves: 100, twigs: 35, grass: 200, bushes: 45, trees: 160, props: 100, rain: 50 },
    QualityPreset { dirt: 40, leaves: 160, twigs: 50, grass: 280, bushes: 60, trees: 210, props: 140, rain: 70 },
    QualityPreset { dirt: 60, leaves: 240, twigs: 70, grass: 380, bushes: 80, trees: 280, props: 200, rain: 100 },
];

#[derive(Clone, Copy, PartialEq, Eq)]
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

#[derive(Resource, Clone)]
pub struct GraphicsSettings {
    pub resolution_idx: usize,
    pub window_mode: WindowModeChoice,
    pub vsync: bool,
    pub fps_cap_idx: usize,
    pub quality_idx: usize,
    pub show_fps: bool,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            resolution_idx: 0,
            window_mode: WindowModeChoice::Borderless,
            vsync: true,
            fps_cap_idx: 0,
            quality_idx: 2,
            show_fps: false,
        }
    }
}

impl GraphicsSettings {
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

    pub fn show_fps_label(&self) -> &'static str {
        if self.show_fps { "ON" } else { "OFF" }
    }

    pub fn quality_preset(&self) -> &'static QualityPreset {
        &QUALITY_PRESETS[self.quality_idx]
    }
}

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GraphicsSettings>()
            .add_systems(Update, (detect_initial_resolution, apply_graphics_settings))
            .add_systems(Last, fps_limiter);
    }
}

fn detect_initial_resolution(
    mut settings: ResMut<GraphicsSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut ran: Local<bool>,
) {
    if *ran {
        return;
    }
    *ran = true;
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

fn fps_limiter(settings: Res<GraphicsSettings>, mut last: Local<Option<Instant>>) {
    let Some(cap) = FPS_CAPS[settings.fps_cap_idx] else {
        *last = None;
        return;
    };
    let target = Duration::from_secs_f64(1.0 / cap as f64);
    let now = Instant::now();
    if let Some(prev) = *last {
        let elapsed = now.saturating_duration_since(prev);
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
    *last = Some(Instant::now());
}
