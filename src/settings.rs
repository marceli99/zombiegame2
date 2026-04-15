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
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            resolution_idx: 0,
            window_mode: WindowModeChoice::Windowed,
            vsync: true,
            fps_cap_idx: 0,
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
}

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GraphicsSettings>()
            .add_systems(Update, apply_graphics_settings)
            .add_systems(Last, fps_limiter);
    }
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
