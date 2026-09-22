//! The application-owned window loop.
//!
//! Why the starter drives its own loop instead of `velqu_shell::run`:
//! the shell forwards input to the view and **drops the event batch**
//! inside its redraw — by design, a display shell observes nothing.
//! The operations workflow needs the opposite: the host must see
//! `Click`/`ValueChanged` events through the public queue so a Save
//! click can trigger host-side validation and persistence
//! (`records.json`). Owning the loop is the application-host concern
//! the reviewer's contract names; the renderer still never touches the
//! disk, and no framework API changed to make this possible —
//! everything below is public `VelquView` + winit + softbuffer +
//! arboard usage.
//!
//! Semantics mirror the framework shell's proven contract: `Wait`
//! control flow (sleep until input), presentation-only repaints on
//! interaction-styling changes, browser-signed wheel deltas, the
//! AltGr-safe keyboard routing table, OS clipboard behind the public
//! `ClipboardProvider` trait, and IME enablement/anchoring through the
//! public composition API. Dev-time concerns (hot reload, inspection)
//! stay in `velqu-lab` via `./dev.sh`.

use std::rc::Rc;
use std::time::{Duration, Instant};

use velqu_view::{
    ClipboardProvider, ControlRect, CursorStyle, Event, FrameResult, KeyCommand, KeyModifiers,
    VelquView, Viewport,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Window};

/// Vertical px per wheel line notch, matching the framework shell.
const WHEEL_LINE_PX: f32 = 40.0;

/// Window configuration for [`run_host_window`].
pub struct HostWindowConfig {
    /// Window title.
    pub title: String,
    /// Initial logical size.
    pub logical_size: (f32, f32),
    /// Whether the user may resize.
    pub resizable: bool,
    /// Auto-close after this duration (smoke tests / measurements).
    pub exit_after: Option<Duration>,
    /// Print `measure:` lines (launch→first presentation, exit stats).
    pub measure: bool,
    /// Process-launch instant; first-presentation latency is measured
    /// from here, not from loop start.
    pub launch: Instant,
}

impl HostWindowConfig {
    /// Config with `title` and a 1280×800 logical window.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            logical_size: (1280.0, 800.0),
            resizable: true,
            exit_after: None,
            measure: false,
            launch: Instant::now(),
        }
    }

    /// Sets the initial logical size.
    pub fn with_logical_size(mut self, width: f32, height: f32) -> Self {
        self.logical_size = (width, height);
        self
    }

    /// Auto-closes after `duration`.
    pub fn with_exit_after(mut self, duration: Duration) -> Self {
        self.exit_after = Some(duration);
        self
    }

    /// Enables measure prints.
    pub fn with_measure(mut self) -> Self {
        self.measure = true;
        self
    }
}

/// What the loop did before the window closed.
#[derive(Debug, Clone, Copy)]
pub struct HostWindowStats {
    /// Frames presented.
    pub frames: u64,
    /// Wall time the window was open.
    pub uptime: Duration,
    /// Launch→first-presented-frame latency, the qualification metric
    /// (distinct from render wall time, which excludes process startup,
    /// document loading, and layout).
    pub first_present: Option<Duration>,
}

/// Host hook run on the main thread each redraw, after the reactive
/// pump and before the frame renders, with the drained event batch.
/// This is where the operations workflow observes Save clicks.
///
/// Returns whether the hook published new application state (a save
/// outcome reloads the presentation document). When true, the loop
/// requests another redraw so the published state is presented even if
/// this redraw's frame was obtained before publication — state changes
/// must never wait for an unrelated input event to become visible
/// (post-save acknowledgement regression, corrective-1). This is an
/// edge-triggered request, not polling: with nothing to publish the
/// loop stays quiescent on `ControlFlow::Wait`.
pub type FrameEventsHook<'a> = Box<dyn FnMut(&mut VelquView, Viewport, &[Event]) -> bool + 'a>;

/// Opens a window and runs the loop until close/exit-after/error.
pub fn run_host_window(
    config: HostWindowConfig,
    view: &mut VelquView,
    hook: Option<FrameEventsHook<'_>>,
) -> Result<HostWindowStats, String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop failed: {e}"))?;
    let context = softbuffer::Context::new(event_loop.owned_display_handle())
        .map_err(|e| format!("surface context failed: {e}"))?;
    // OS clipboard when the session has one; the view keeps its null
    // provider otherwise (paste reads nothing, cut stays transactional).
    let clipboard: Option<Rc<dyn ClipboardProvider>> =
        arboard::Clipboard::new().ok().map(|board| {
            let provider: Rc<dyn ClipboardProvider> = Rc::new(ArboardClipboard {
                board: std::cell::RefCell::new(board),
            });
            view.set_clipboard_provider(provider.clone());
            provider
        });

    let exit_at = config.exit_after.map(|d| Instant::now() + d);
    event_loop.set_control_flow(match exit_at {
        Some(at) => ControlFlow::WaitUntil(at),
        None => ControlFlow::Wait,
    });

    let mut app = HostApp {
        config,
        view,
        context,
        window: None,
        surface: None,
        dirty: true,
        // NaN until the first CursorMoved: containment comparisons are
        // false, so early presses hit nothing.
        pointer: (f32::NAN, f32::NAN),
        last_cursor: None,
        frames: 0,
        first_present: None,
        modifiers: ModifiersState::default(),
        ime_allowed: false,
        ime_rect: None,
        error: None,
        hook,
    };
    let loop_result = event_loop.run_app(&mut app);
    let stats = app.take_result();
    // Explicit clipboard teardown (arboard lifecycle on Wayland/X11:
    // this process is the clipboard owner).
    if clipboard.is_some() {
        view.set_clipboard_provider(Rc::new(velqu_view::NullClipboardProvider));
    }
    drop(clipboard);
    loop_result.map_err(|e| format!("event loop failed: {e}"))?;
    stats
}

struct HostApp<'a> {
    config: HostWindowConfig,
    view: &'a mut VelquView,
    context: softbuffer::Context<winit::event_loop::OwnedDisplayHandle>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<winit::event_loop::OwnedDisplayHandle, Rc<Window>>>,
    dirty: bool,
    /// Last pointer position in physical window px.
    pointer: (f32, f32),
    last_cursor: Option<CursorStyle>,
    frames: u64,
    first_present: Option<Duration>,
    modifiers: ModifiersState,
    ime_allowed: bool,
    ime_rect: Option<ControlRect>,
    error: Option<String>,
    hook: Option<FrameEventsHook<'a>>,
}

impl HostApp<'_> {
    fn take_result(self) -> Result<HostWindowStats, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(HostWindowStats {
            frames: self.frames,
            uptime: self.config.launch.elapsed(),
            first_present: self.first_present,
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn setup(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.logical_size.0,
                self.config.logical_size.1,
            ))
            .with_resizable(self.config.resizable);
        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("window creation failed: {e}"))?,
        );
        let surface = softbuffer::Surface::new(&self.context, window.clone())
            .map_err(|e| format!("surface failed: {e}"))?;
        window.request_redraw();
        self.window = Some(window);
        self.surface = Some(surface);
        self.dirty = true;
        Ok(())
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// The viewport matching the window, or `None` before the window
    /// exists / while minimized (input is dropped in both cases).
    fn viewport(&self) -> Option<Viewport> {
        let window = self.window.as_ref()?;
        let physical = window.inner_size();
        Viewport::try_new(
            physical.width.max(1),
            physical.height.max(1),
            window.scale_factor() as f32,
        )
        .ok()
    }

    fn pointer_moved(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        let Some(viewport) = self.viewport() else {
            return;
        };
        self.pointer = (position.x as f32, position.y as f32);
        if self
            .view
            .pointer_move(viewport, self.pointer.0, self.pointer.1)
        {
            self.dirty = true;
        }
        let cursor = self
            .view
            .cursor_under(viewport, self.pointer.0, self.pointer.1);
        if self.last_cursor != Some(cursor) {
            self.last_cursor = Some(cursor);
            if let Some(window) = &self.window {
                window.set_cursor(cursor_icon(cursor));
            }
        }
    }

    /// Pushes IME enablement + candidate anchor to the window, both
    /// change-gated (public composition API).
    fn update_ime(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let allowed = self.view.wants_ime();
        if allowed != self.ime_allowed {
            self.ime_allowed = allowed;
            window.set_ime_allowed(allowed);
        }
        if !allowed {
            self.ime_rect = None;
            return;
        }
        let Some(viewport) = self.viewport() else {
            return;
        };
        let Some(rect) = self.view.ime_cursor_rect(viewport) else {
            return;
        };
        if self.ime_rect != Some(rect) {
            self.ime_rect = Some(rect);
            window.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(rect.x.round() as i32, rect.y.round() as i32),
                winit::dpi::PhysicalSize::new(
                    rect.width.round().max(1.0) as u32,
                    rect.height.round().max(1.0) as u32,
                ),
            );
        }
    }

    fn ime_event(&mut self, event: Ime) {
        let changed = match event {
            Ime::Preedit(text, cursor) => self.view.ime_preedit(&text, cursor),
            Ime::Commit(text) => self.view.ime_commit(&text),
            Ime::Disabled => self.view.ime_cancel(),
            Ime::Enabled => false,
        };
        if changed {
            self.dirty = true;
            self.request_redraw();
        }
        self.update_ime();
    }

    fn key_modifiers(&self) -> KeyModifiers {
        KeyModifiers {
            ctrl: self.modifiers.control_key(),
            command: self.modifiers.super_key(),
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
        }
    }

    fn keyboard_input(&mut self, event_loop: &ActiveEventLoop, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let modifiers = self.key_modifiers();
        match route_keyboard_input(
            &event.logical_key,
            event.physical_key,
            modifiers,
            event.text.as_deref(),
        ) {
            KeyboardRoute::Command(KeyCommand::Escape) => event_loop.exit(),
            KeyboardRoute::Command(KeyCommand::Tab) => {
                if !event.repeat {
                    self.view.focus_next();
                    self.dirty = true;
                    self.request_redraw();
                }
            }
            KeyboardRoute::Command(command) => {
                if self.view.key_command(command, modifiers) {
                    self.dirty = true;
                    self.request_redraw();
                }
            }
            KeyboardRoute::InsertText(text) => {
                if self.view.insert_text(text) {
                    self.dirty = true;
                    self.request_redraw();
                }
            }
            KeyboardRoute::Ignored => {}
        }
    }

    /// One frame: drain events → pump reactive → **host hook inspects
    /// the batch** (the reason this loop exists) → render → present.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.redraw_inner() {
            self.fail(event_loop, error);
        }
    }

    fn redraw_inner(&mut self) -> Result<(), String> {
        let viewport = match self.viewport() {
            Some(viewport) => viewport,
            None => {
                // Still pump so reactive turns are not starved, but
                // there is nothing to present.
                let batch = self.view.take_events();
                self.view.pump_reactive(&batch);
                return Ok(());
            }
        };
        let batch = self.view.take_events();
        self.view.pump_reactive(&batch);
        if std::env::var_os("VELQU_TRACE_PRESENT").is_some() && !batch.is_empty() {
            eprintln!(
                "trace: redraw batch={} gen_before={}",
                batch.len(),
                self.view.reactive_plan().map(|_| 0).unwrap_or(0)
            );
            for event in &batch {
                let kind = match event {
                    Event::Click { target } => format!("Click({})", target.id.as_deref().unwrap_or("?")),
                    Event::ValueChanged { target, .. } => {
                        format!("ValueChanged({})", target.id.as_deref().unwrap_or("?"))
                    }
                    other => format!("{other:?}"),
                };
                eprintln!("trace:   {kind}");
            }
        }
        if let Some(hook) = self.hook.as_mut() {
            if hook(self.view, viewport, &batch) {
                // New application state exists; guarantee it reaches the
                // window (edge-triggered, no polling).
                self.dirty = true;
                self.request_redraw();
            }
        }
        let FrameResult { frame, .. } = self
            .view
            .render(viewport)
            .map_err(|e| format!("render failed: {e}"))?;
        if std::env::var_os("VELQU_TRACE_PRESENT").is_some() {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash_slice(frame.pixels(), &mut hasher);
            use std::hash::Hasher;
            eprintln!(
                "trace: rendered frame hash={:016x} ({} px), presenting",
                std::hash::Hasher::finish(&hasher),
                frame.pixels().len() / 4
            );
        }

        let Some(window) = self.window.clone() else {
            return Ok(());
        };
        let physical = window.inner_size();
        let (Some(width), Some(height)) = (
            std::num::NonZeroU32::new(physical.width),
            std::num::NonZeroU32::new(physical.height),
        ) else {
            return Ok(()); // minimized
        };
        let surface = self
            .surface
            .as_mut()
            .ok_or("redraw before the surface existed")?;
        surface
            .resize(width, height)
            .map_err(|e| format!("surface resize failed: {e}"))?;
        let mut buffer = surface
            .buffer_mut()
            .map_err(|e| format!("surface buffer failed: {e}"))?;
        for (dst, rgba) in buffer.iter_mut().zip(frame.pixels().chunks_exact(4)) {
            // softbuffer's buffer format is native-endian 0x00RRGGBB.
            *dst = u32::from_be_bytes([0, rgba[0], rgba[1], rgba[2]]);
        }
        buffer
            .present()
            .map_err(|e| format!("present failed: {e}"))?;
        self.frames += 1;
        if self.first_present.is_none() {
            self.first_present = Some(self.config.launch.elapsed());
            if self.config.measure {
                let ms = self.first_present.map(|d| d.as_millis()).unwrap_or(0);
                println!(
                    "measure: first_present_ms={ms} viewport={}x{}@{:.2}",
                    viewport.width(),
                    viewport.height(),
                    viewport.scale_factor()
                );
            }
        }
        Ok(())
    }
}

impl ApplicationHandler<()> for HostApp<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.setup(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) => {
                self.dirty = true;
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.dirty = true;
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.keyboard_input(event_loop, &event);
                self.update_ime();
            }
            WindowEvent::Ime(event) => self.ime_event(event),
            WindowEvent::Focused(focused) => {
                if !focused {
                    self.view.ime_cancel();
                }
                self.update_ime();
            }
            WindowEvent::CursorMoved { position, .. } => self.pointer_moved(position),
            WindowEvent::CursorLeft { .. } => {
                self.view.pointer_exit();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(viewport) = self.viewport() else {
                    return;
                };
                if button != MouseButton::Left {
                    return;
                }
                let (x, y) = self.pointer;
                let changed = match state {
                    ElementState::Pressed => self.view.pointer_press(viewport, x, y),
                    ElementState::Released => self.view.pointer_release(viewport, x, y),
                };
                if changed {
                    self.dirty = true;
                    self.request_redraw();
                }
                self.update_ime();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(viewport) = self.viewport() else {
                    return;
                };
                // Browser-signed delta (positive dy = scroll forward);
                // winit's is opposite, and line deltas scale by the
                // wheel line height.
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * WHEEL_LINE_PX, -y * WHEEL_LINE_PX),
                    MouseScrollDelta::PixelDelta(position) => {
                        (-(position.x as f32), -(position.y as f32))
                    }
                };
                let (px, py) = self.pointer;
                self.view.wheel(viewport, px, py, dx, dy);
                self.dirty = true;
                self.request_redraw();
                self.update_ime();
            }
            WindowEvent::RedrawRequested => {
                self.redraw(event_loop);
                self.dirty = false;
                self.update_ime();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(exit_after) = self.config.exit_after {
            if self.config.launch.elapsed() >= exit_after {
                event_loop.exit();
            }
        }
        if self.dirty {
            self.request_redraw();
        }
    }
}

/// The routing decision for one keyboard input (pure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyboardRoute<'a> {
    Command(KeyCommand),
    InsertText(&'a str),
    Ignored,
}

/// Named keys are commands before any text consideration (winit gives
/// Enter the text `"\r"`); exact Ctrl/Cmd+A/C/X/V chords are commands;
/// Ctrl/Cmd without Alt suppresses text; everything else inserts the
/// platform's produced text. Alt disqualifies both the chord and the
/// suppression, so AltGr chords (Ctrl+Alt on international layouts)
/// still type their glyph.
fn route_keyboard_input<'a>(
    logical_key: &Key,
    physical_key: PhysicalKey,
    modifiers: KeyModifiers,
    text: Option<&'a str>,
) -> KeyboardRoute<'a> {
    if let Some(command) = named_key_command(logical_key) {
        return KeyboardRoute::Command(command);
    }
    if let Some(command) = shortcut_chord(logical_key, physical_key, modifiers) {
        return KeyboardRoute::Command(command);
    }
    if (modifiers.ctrl || modifiers.command) && !modifiers.alt {
        return KeyboardRoute::Ignored;
    }
    match text {
        Some(text) if !text.is_empty() => KeyboardRoute::InsertText(text),
        _ => KeyboardRoute::Ignored,
    }
}

fn named_key_command(logical_key: &Key) -> Option<KeyCommand> {
    match logical_key {
        Key::Named(NamedKey::Backspace) => Some(KeyCommand::Backspace),
        Key::Named(NamedKey::Delete) => Some(KeyCommand::Delete),
        Key::Named(NamedKey::ArrowLeft) => Some(KeyCommand::Left),
        Key::Named(NamedKey::ArrowRight) => Some(KeyCommand::Right),
        Key::Named(NamedKey::ArrowUp) => Some(KeyCommand::Up),
        Key::Named(NamedKey::ArrowDown) => Some(KeyCommand::Down),
        Key::Named(NamedKey::Home) => Some(KeyCommand::Home),
        Key::Named(NamedKey::End) => Some(KeyCommand::End),
        Key::Named(NamedKey::Enter) => Some(KeyCommand::Enter),
        Key::Named(NamedKey::Tab) => Some(KeyCommand::Tab),
        Key::Named(NamedKey::Escape) => Some(KeyCommand::Escape),
        _ => None,
    }
}

fn shortcut_chord(
    logical_key: &Key,
    physical_key: PhysicalKey,
    modifiers: KeyModifiers,
) -> Option<KeyCommand> {
    if (!modifiers.ctrl && !modifiers.command) || modifiers.alt {
        return None;
    }
    let letter = |target: &str, code: KeyCode| {
        matches!(logical_key, Key::Character(text) if text.eq_ignore_ascii_case(target))
            || matches!(physical_key, PhysicalKey::Code(actual) if actual == code)
    };
    if letter("a", KeyCode::KeyA) {
        Some(KeyCommand::SelectAll)
    } else if letter("c", KeyCode::KeyC) {
        Some(KeyCommand::Copy)
    } else if letter("x", KeyCode::KeyX) {
        Some(KeyCommand::Cut)
    } else if letter("v", KeyCode::KeyV) {
        Some(KeyCommand::Paste)
    } else {
        None
    }
}

/// OS clipboard adapter behind the public renderer trait; platform
/// failures surface as `Err` so cut/copy stay transactional.
struct ArboardClipboard {
    board: std::cell::RefCell<arboard::Clipboard>,
}

impl ClipboardProvider for ArboardClipboard {
    fn read(&self) -> Result<Option<String>, velqu_view::ClipboardError> {
        self.board
            .borrow_mut()
            .get_text()
            .map(Some)
            .map_err(|error| velqu_view::ClipboardError::new(error.to_string()))
    }

    fn write(&self, text: &str) -> Result<(), velqu_view::ClipboardError> {
        self.board
            .borrow_mut()
            .set_text(text.to_owned())
            .map_err(|error| velqu_view::ClipboardError::new(error.to_string()))
    }
}

fn cursor_icon(style: CursorStyle) -> CursorIcon {
    match style {
        CursorStyle::Auto | CursorStyle::Default => CursorIcon::Default,
        CursorStyle::Pointer => CursorIcon::Pointer,
        CursorStyle::Text => CursorIcon::Text,
    }
}
