//! The desktop app runner with the connected monitors. Meant for `ui-shell/src/shell/app.rs` (craft-ui-shell) and
//! `window/src/system.rs` (craft-window).
//!
//! craft's `app::run` opens the root window where its placement was saved before any monitor is known, and craft's
//! `SystemWindows` lists no monitors, so nothing moves a window back that was saved on a monitor since unplugged, and a
//! floating dock window is placed against no monitor either. The runner keeps eframe's window to itself, so the app
//! cannot list them. [`run`] is the same runner over craft's `embed` hooks: it takes the monitors from eframe's window,
//! moves the root window back onto one when it opened out of reach, and gives the window host a [`WindowSystem`] that
//! lists them. In craft, `SystemWindows` would list them and `app::run` would place the window again.

use std::error::Error;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use craft::behavior::monitor::{Monitor, MonitorKey};
use craft::behavior::placement::{Defaults, Placement, place};
use craft::ui::InstallSpec;
use craft::ui::fonts::{I18nScripts, UiFonts};
use craft::ui::runtime::env::SystemReader;
use craft::ui::runtime::frame::request_repaint_after;
use craft::ui::runtime::motion;
use craft::ui::shell::app::{AppSpec, ShellApp, native_options};
use craft::ui::shell::embed;
use craft::window::restore::MAIN;
use craft::window::system::WindowSystem;
use egui::Context;

/// Most monitors listed.
const MAX_MONITORS: usize = 64;
/// How often the monitors are listed again while the app draws.
const LIST_AGAIN: Duration = Duration::from_secs(2);
/// Largest window side asked for, in pixels.
const MAX_SIDE_PX: f32 = 32_768.0;

/// Opens the root window with `icon` and runs `make`'s app until it closes, as craft's `app::run` does, with the
/// connected monitors known.
pub fn run<A: ShellApp + 'static>(spec: AppSpec, icon: Option<egui::IconData>, make: impl FnOnce(&Context) -> A + 'static) -> eframe::Result {
    let mut options = native_options(&spec);
    if let Some(icon) = icon {
        options.viewport = options.viewport.with_icon(icon);
    }
    let title = spec.app_id.clone();
    eframe::run_native(&title, options, Box::new(create(spec, make)))
}

/// Where the root window has to go when the saved placement it opened with is out of reach on `monitors` or too large
/// for them; `None` when it can stay. It opened as craft's `native_options` places it: with no monitor known.
pub fn back_on_screen(saved: Option<&Placement>, monitors: &[Monitor], defaults: &Defaults) -> Option<Placement> {
    let saved = saved?;
    if monitors.is_empty() {
        return None;
    }
    let opened = place(Some(saved), &[], None, defaults);
    let placed = place(Some(saved), monitors, None, defaults);
    (placed.position_px != opened.position_px || placed.normal.size != opened.normal.size).then_some(placed)
}

/// The OS services of `inner`, with the connected monitors listed by the app runner.
struct ListedMonitors {
    inner: Arc<dyn WindowSystem>,
    listed: Mutex<Vec<Monitor>>,
}

impl ListedMonitors {
    fn list(&self) -> MutexGuard<'_, Vec<Monitor>> {
        self.listed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WindowSystem for ListedMonitors {
    fn desktop_pointer(&self) -> Option<[f32; 2]> {
        self.inner.desktop_pointer()
    }

    fn primary_held(&self) -> Option<bool> {
        self.inner.primary_held()
    }

    fn command_held(&self) -> Option<bool> {
        self.inner.command_held()
    }

    fn warp_pointer(&self, to: [f32; 2]) -> bool {
        self.inner.warp_pointer(to)
    }

    fn monitors(&self) -> Vec<Monitor> {
        let own = self.inner.monitors();
        if own.is_empty() { self.list().clone() } else { own }
    }
}

/// Lists the connected monitors; nothing once eframe's window is gone.
type ListMonitors = Box<dyn Fn() -> Vec<Monitor>>;

/// eframe's `App` over a `ShellApp`, as craft's own driver is, which also keeps the monitors listed.
struct Driver<A> {
    app: A,
    monitors: Arc<ListedMonitors>,
    list: ListMonitors,
    listed_at: Instant,
}

impl<A: ShellApp> eframe::App for Driver<A> {
    fn raw_input_hook(&mut self, ctx: &Context, _: &mut egui::RawInput) {
        embed::before_input(ctx, |output| {
            self.app.output(output);
            self.app.frame_input()
        });
    }

    fn logic(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        if self.listed_at.elapsed() >= LIST_AGAIN {
            self.listed_at = Instant::now();
            *self.monitors.list() = (self.list)();
        }
        self.app.logic(ctx);
        let now = motion::now();
        if let Some(due) = self.app.tick(now) {
            request_repaint_after(ctx, due.saturating_duration_since(now).as_secs_f64());
        }
        embed::logic(ctx, || self.app.close_requested());
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        embed::before_ui(ui.ctx(), frame);
        self.app.ui(ui);
    }
}

/// Installs the runtime on eframe's context, puts the root window within reach, then builds the app with `make`.
fn create<A: ShellApp + 'static>(
    spec: AppSpec,
    make: impl FnOnce(&Context) -> A + 'static,
) -> impl FnOnce(&eframe::CreationContext<'_>) -> Result<Box<dyn eframe::App>, Box<dyn Error + Send + Sync>> {
    move |cc| {
        let list: ListMonitors = match cc.winit_window() {
            Some(window) => {
                let window = Arc::downgrade(window);
                Box::new(move || {
                    let Some(window) = window.upgrade() else { return Vec::new() };
                    let monitors = window.available_monitors().take(MAX_MONITORS).map(|monitor| {
                        let (at, size) = (monitor.position(), monitor.size());
                        let size = [size.width, size.height];
                        let key = MonitorKey::new(monitor.name().unwrap_or_default(), size);
                        Monitor::new(key, [at.x, at.y], size, monitor.scale_factor() as f32)
                    });
                    // A monitor of no size (one being connected) holds no window.
                    monitors.filter(|monitor| monitor.size_px.iter().all(|side| *side > 0)).collect()
                })
            }
            None => Box::new(Vec::<Monitor>::new),
        };
        let listed = list();
        let saved = spec.placement.lock().unwrap_or_else(PoisonError::into_inner).get(MAIN);
        if let Some(window) = cc.winit_window()
            && let Some(placed) = back_on_screen(saved.as_ref(), &listed, &spec.defaults)
        {
            let monitor = window.available_monitors().next();
            if let (Some([x, y]), Some(monitor)) = (placed.position_px, &monitor) {
                let mut at = monitor.position();
                (at.x, at.y) = (x, y);
                window.set_outer_position(at);
            }
            if placed.maximized {
                // Moving a maximized window restores it on some platforms.
                window.set_maximized(true);
            } else if let Some(monitor) = &monitor {
                let px = placed.normal.size.map(|side| (side * placed.scale).round().clamp(1.0, MAX_SIDE_PX) as u32);
                let mut size = monitor.size();
                [size.width, size.height] = px;
                // The window reports the size it got.
                let _ = window.request_inner_size(size);
            }
        }
        let monitors = Arc::new(ListedMonitors { inner: spec.window_system, listed: Mutex::new(listed) });
        let ctx = cc.egui_ctx.clone();
        let system = SystemReader::start(move || ctx.request_repaint());
        let window_system: Arc<dyn WindowSystem> = Arc::clone(&monitors) as Arc<dyn WindowSystem>;
        let mut install = InstallSpec::new(spec.shell_info, window_system).placement(spec.placement).system(system).app_id(spec.app_id);
        if let Some(decision) = spec.frame {
            install = install.frame(decision);
        }
        if spec.stories {
            install = install.stories();
        }
        // The suite's interface face; an app that installs its own fonts in `make` replaces it.
        let fonts = UiFonts::new(&I18nScripts);
        embed::install(&cc.egui_ctx, install, &fonts).map_err(|error| format!("the runtime could not be installed: {error}"))?;
        Ok(Box::new(Driver { app: make(&cc.egui_ctx), monitors, list, listed_at: Instant::now() }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn monitor(name: &str, x: i32, size: [u32; 2]) -> Monitor {
        Monitor::new(MonitorKey::new(name, size), [x, 0], size, 1.0)
    }

    /// A window saved on a second monitor stays there while that monitor is connected, and comes back onto the one
    /// that is left when it is not; a window larger than its monitor is fitted to it.
    #[test]
    fn a_window_saved_off_every_monitor_comes_back_within_reach() {
        let (first, second) = (monitor("A", 0, [1920, 1080]), monitor("B", 1920, [1920, 1080]));
        let defaults = Defaults::new([1280.0, 800.0], [640.0, 480.0]);
        let mut saved = Placement::new([1000.0, 700.0], 1.0);
        saved.position_px = Some([2400, 100]);
        saved.monitor = Some(second.key.clone());
        assert_eq!(back_on_screen(Some(&saved), &[first.clone(), second], &defaults), None);

        let back = back_on_screen(Some(&saved), std::slice::from_ref(&first), &defaults).unwrap();
        let [x, y] = back.position_px.unwrap();
        assert!((0..=920).contains(&x) && (0..=380).contains(&y), "{x}, {y}");
        assert_eq!(back.normal.size, [1000.0, 700.0]);

        saved.position_px = Some([10, 10]);
        saved.normal.size = [4000.0, 3000.0];
        let fitted = back_on_screen(Some(&saved), &[first], &defaults).unwrap();
        assert!(fitted.normal.size[0] <= 1920.0 && fitted.normal.size[1] <= 1080.0, "{:?}", fitted.normal.size);

        assert_eq!(back_on_screen(Some(&saved), &[], &defaults), None, "no monitor is listed: nothing can be checked");
        assert_eq!(back_on_screen(None, &[monitor("A", 0, [800, 600])], &defaults), None);
    }
}
