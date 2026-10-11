//! The macOS menu bar item. A click opens a panel under it with a live
//! picture of a machine's screen and a few actions. The picture is the
//! machine's session as the window shows it, so clicking the picture opens
//! that session in the window. The status item and the panel's placement use
//! AppKit; the panel itself is a GTK window.

use std::cell::RefCell;
use std::ffi::{c_void, OsStr};
use std::rc::Rc;

use gliff_transport::SshTarget;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4 as gtk;
use libadwaita as adw;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSImage, NSPopUpMenuWindowLevel, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, NSObject, NSPoint};

use crate::net::Endpoint;
use crate::recent::Config;
use crate::{
    active, find_session, refresh_tabs, show_session, ssh_endpoint, start_session, stop_session,
    App, Session,
};

/// Width of the live picture, in logical pixels.
const PREVIEW_WIDTH: i32 = 480;

/// Runs before a remote action. A plain ssh login has no Hyprland session
/// environment, so it is taken from the newest Hyprland instance that is
/// still running, as `hypr-ipc` finds it.
const SESSION_ENV: &str = r#"export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
for d in $(ls -t "$XDG_RUNTIME_DIR/hypr" 2>/dev/null); do
  l="$XDG_RUNTIME_DIR/hypr/$d/hyprland.lock"
  if [ -f "$l" ] && kill -0 "$(head -n 1 "$l")" 2>/dev/null; then
    export HYPRLAND_INSTANCE_SIGNATURE="$d" WAYLAND_DISPLAY="$(sed -n 2p "$l")"
    break
  fi
done
"#;

struct Menu {
    ui: Rc<App>,
    item: Retained<NSStatusItem>,
    _target: Retained<ClickTarget>,
    panel: gtk::Window,
    title: gtk::Label,
    picture: gtk::Picture,
    status: gtk::Label,
    /// The outcome of the last action.
    note: gtk::Label,
    /// The machine the panel shows.
    machine: RefCell<Option<String>>,
}

thread_local! {
    static MENU: RefCell<Option<Rc<Menu>>> = const { RefCell::new(None) };
}

fn menu() -> Option<Rc<Menu>> {
    MENU.with_borrow(Clone::clone)
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and ClickTarget does
    // not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GliffMenuBarTarget"]
    struct ClickTarget;

    impl ClickTarget {
        #[unsafe(method(clicked:))]
        fn clicked(&self, _sender: Option<&AnyObject>) {
            // GTK is touched after AppKit's event dispatch returns.
            glib::idle_add_local_once(toggle);
        }
    }
);

impl ClickTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's `init` takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

/// Put the item in the menu bar.
pub fn install(ui: &Rc<App>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let target = ClickTarget::new(mtm);
    if let Some(button) = item.button(mtm) {
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            ns_string!("display"),
            Some(ns_string!("gliff")),
        );
        match image {
            Some(image) => {
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
            None => button.setTitle(ns_string!("gliff")),
        }
        // SAFETY: the target defines `clicked:` with one object argument,
        // and MENU keeps the target alive as long as the item.
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(clicked:)));
        }
    }
    let menu = build_panel(ui, item, target);
    MENU.set(Some(Rc::new(menu)));
}

fn build_panel(ui: &Rc<App>, item: Retained<NSStatusItem>, target: Retained<ClickTarget>) -> Menu {
    let title = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .css_classes(["heading"])
        .build();
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .width_request(PREVIEW_WIDTH)
        .height_request(PREVIEW_WIDTH * 9 / 16)
        .css_classes(["video"])
        .build();
    let status = gtk::Label::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .wrap(true)
        .can_target(false)
        .css_classes(["status-message"])
        .build();
    // A picture's natural size is the frame's, so the clamp keeps the panel
    // from growing to the remote screen's full resolution.
    let clamp = adw::Clamp::builder()
        .maximum_size(PREVIEW_WIDTH)
        .tightening_threshold(PREVIEW_WIDTH)
        .child(&picture)
        .build();
    let screen = gtk::Overlay::builder()
        .child(&clamp)
        .tooltip_text("Open in the window")
        .build();
    screen.add_overlay(&status);
    screen.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    click.connect_released(|_, _, _, _| open_window());
    screen.add_controller(click);

    let actions = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let entries: [(&str, fn()); 5] = [
        ("Open private screen", open_private),
        ("Lock screen", lock),
        ("Screenshot to clipboard", screenshot),
        ("Disconnect", disconnect),
        ("Quit gliff", quit),
    ];
    for (label, run) in entries {
        let text = gtk::Label::builder().label(label).xalign(0.0).build();
        let button = gtk::Button::builder()
            .child(&text)
            .css_classes(["flat"])
            .build();
        button.connect_clicked(move |_| run());
        actions.append(&button);
    }
    let note = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .wrap(true)
        .visible(false)
        .css_classes(["dim-label"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(10)
        .margin_end(10)
        .build();
    content.append(&title);
    content.append(&screen);
    content.append(&actions);
    content.append(&note);
    let panel = gtk::Window::builder()
        .title("Gliff menu bar")
        .decorated(false)
        .resizable(false)
        .hide_on_close(true)
        .child(&content)
        .build();
    if let Some(app) = ui.window.application() {
        panel.set_application(Some(&app));
    }
    // GTK gives every toplevel a title bar mask, which makes window managers
    // such as AeroSpace tile the panel. Without one it is a popup to them;
    // set before the first show, when they first see the window.
    WidgetExt::realize(&panel);
    if let Some(window) = ns_window(&panel) {
        window.setStyleMask(NSWindowStyleMask::Borderless);
        window.setLevel(NSPopUpMenuWindowLevel);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::Transient | NSWindowCollectionBehavior::IgnoresCycle,
        );
    }
    // AppKit keeps a window's bottom edge when it grows, so a taller panel
    // (a first frame, a note) would rise into the menu bar.
    if let Some(surface) = panel.surface() {
        surface.connect_layout(|_, _, _| {
            if let Some(m) = menu().filter(|m| m.panel.is_visible()) {
                place(&m);
            }
        });
    }
    // Like a menu, the panel goes away when anything else is clicked.
    panel.connect_is_active_notify(|w| {
        if !w.is_active() {
            w.set_visible(false);
        }
    });
    Menu {
        ui: ui.clone(),
        item,
        _target: target,
        panel,
        title,
        picture,
        status,
        note,
        machine: RefCell::new(None),
    }
}

/// Open the panel under the item, or close it.
fn toggle() {
    let Some(m) = menu() else {
        return;
    };
    if m.panel.is_visible() {
        m.panel.set_visible(false);
        return;
    }
    let machine = target(&m.ui);
    m.note.set_visible(false);
    match &machine {
        Some(name) => {
            let s = session_for(&m.ui, name);
            m.title.set_text(name);
            let frame = s.last_frame.borrow().as_ref().map(|(t, _)| t.clone());
            set_picture(&m, frame.as_ref());
            set_status(&m, &s.status.borrow());
        }
        None => {
            m.title.set_text("No machine");
            set_picture(&m, None);
            set_status(&m, "Connect to a machine in the window first");
        }
    }
    *m.machine.borrow_mut() = machine;
    if let Some(mtm) = MainThreadMarker::new() {
        // `activate` only asks, and macOS refuses it from a menu bar click,
        // so the panel would never become active and never close.
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
    m.panel.present();
    place(&m);
}

/// The machine the panel shows: the window's, else the one shown last, else
/// the first remembered one.
fn target(ui: &App) -> Option<String> {
    if let Some(s) = active(ui).filter(|s| !s.private) {
        return Some(s.name.clone());
    }
    let config = Config::load(&ui.config_path);
    config.shown.or_else(|| config.machines.into_iter().next())
}

/// The running session for `name`, started if needed. A new one joins the
/// tabs but does not take the window from the session it shows.
fn session_for(ui: &Rc<App>, name: &str) -> Rc<Session> {
    if let Some(s) = find_session(ui, name) {
        if s.gave_up.get() {
            s.retries.set(0);
            s.gave_up.set(false);
            start_session(ui.clone(), s.clone());
        }
        return s;
    }
    let s = Rc::new(Session::new(ssh_endpoint(&ui.cli, name)));
    ui.sessions.borrow_mut().push(s.clone());
    start_session(ui.clone(), s.clone());
    if active(ui).is_none() {
        show_session(ui, &s);
    } else {
        refresh_tabs(ui);
    }
    s
}

/// The panel's machine, while the panel shows `s`.
fn showing(s: &Session) -> Option<Rc<Menu>> {
    let m = menu()?;
    let shows = m.panel.is_visible() && m.machine.borrow().as_deref() == Some(s.name.as_str());
    if shows {
        Some(m)
    } else {
        None
    }
}

/// A session decoded a frame.
pub fn frame(s: &Session, texture: &gdk::Texture) {
    if let Some(m) = showing(s) {
        set_picture(&m, Some(texture));
    }
}

/// A session's status changed.
pub fn status(s: &Session) {
    if let Some(m) = showing(s) {
        set_status(&m, &s.status.borrow());
    }
}

fn set_picture(m: &Menu, texture: Option<&gdk::Texture>) {
    m.picture.set_paintable(texture);
    if let Some(t) = texture.filter(|t| t.width() > 0) {
        let height = PREVIEW_WIDTH * t.height() / t.width();
        m.picture.set_size_request(
            PREVIEW_WIDTH,
            height.clamp(PREVIEW_WIDTH / 4, PREVIEW_WIDTH),
        );
    }
}

fn set_status(m: &Menu, text: &str) {
    m.status.set_text(text);
    m.status.set_visible(!text.is_empty());
}

fn note(m: &Menu, text: &str) {
    m.note.set_text(text);
    m.note.set_visible(true);
}

fn open_window() {
    let Some(m) = menu() else {
        return;
    };
    m.panel.set_visible(false);
    let name = m.machine.borrow().clone();
    if let Some(s) = name.and_then(|n| find_session(&m.ui, &n)) {
        show_session(&m.ui, &s);
    }
    m.ui.window.present();
}

/// Show the machine's private screen in the window: a headless output on
/// the remote, sized to the window, in a tab of its own.
fn open_private() {
    let Some(m) = menu() else {
        return;
    };
    let Some(name) = m.machine.borrow().clone() else {
        return;
    };
    let private_name = format!("{name} (private)");
    let s = match find_session(&m.ui, &private_name) {
        Some(s) => s,
        None => {
            let Some(Endpoint::Ssh(mut t)) = find_session(&m.ui, &name).map(|s| s.endpoint.clone())
            else {
                note(&m, "A private screen needs an ssh machine");
                return;
            };
            t.server_args = vec!["--headless".into()];
            let s = Rc::new(Session {
                name: private_name,
                private: true,
                ..Session::new(Endpoint::Ssh(t))
            });
            m.ui.sessions.borrow_mut().push(s.clone());
            start_session(m.ui.clone(), s.clone());
            s
        }
    };
    m.panel.set_visible(false);
    show_session(&m.ui, &s);
    m.ui.window.present();
}

fn lock() {
    let Some((m, target)) = ssh_target() else {
        return;
    };
    note(&m, "Locking…");
    glib::spawn_future_local(async move {
        let result = run_remote(&target, "omarchy system lock").await;
        if let Some(m) = menu() {
            match result {
                Ok(_) => note(&m, "Locked"),
                Err(e) => note(&m, &format!("Lock failed: {e}")),
            }
        }
    });
}

/// Copy a full-quality screenshot of the shown screen to the clipboard.
fn screenshot() {
    let Some((m, target)) = ssh_target() else {
        return;
    };
    let output = m
        .machine
        .borrow()
        .as_deref()
        .and_then(|n| find_session(&m.ui, n))
        .map(|s| s.output.borrow().clone())
        .unwrap_or_default();
    // The name goes into a shell command.
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_.".contains(c);
    if output.is_empty() || !output.chars().all(plain) {
        note(&m, "A screenshot needs a connected screen");
        return;
    }
    note(&m, "Taking a screenshot…");
    glib::spawn_future_local(async move {
        let result = run_remote(&target, &format!("grim -o {output} -"))
            .await
            .and_then(|png| gdk::Texture::from_bytes(&png).map_err(|e| e.to_string()));
        let Some(m) = menu() else {
            return;
        };
        match result {
            Ok(texture) => {
                m.panel.clipboard().set_texture(&texture);
                note(
                    &m,
                    &format!(
                        "Screenshot of {output} copied ({}×{})",
                        texture.width(),
                        texture.height()
                    ),
                );
            }
            Err(e) => note(&m, &format!("Screenshot failed: {e}")),
        }
    });
}

/// End every session.
fn disconnect() {
    let Some(m) = menu() else {
        return;
    };
    let names: Vec<String> =
        m.ui.sessions
            .borrow()
            .iter()
            .map(|s| s.name.clone())
            .collect();
    for name in names {
        stop_session(&m.ui, &name);
    }
    set_picture(&m, None);
    set_status(&m, "Disconnected");
}

fn quit() {
    if let Some(app) = menu().and_then(|m| m.ui.window.application()) {
        app.quit();
    }
}

/// The panel and the ssh target of the machine it shows.
fn ssh_target() -> Option<(Rc<Menu>, SshTarget)> {
    let m = menu()?;
    let name = m.machine.borrow().clone()?;
    match ssh_endpoint(&m.ui.cli, &name) {
        Endpoint::Ssh(t) => Some((m, t)),
        Endpoint::Tcp(_) => None,
    }
}

/// Run a shell script on the machine over ssh, in its Hyprland session's
/// environment, and return what it prints. The script goes in through
/// stdin, so nothing needs quoting.
async fn run_remote(target: &SshTarget, script: &str) -> Result<glib::Bytes, String> {
    let mut argv: Vec<&OsStr> = vec![OsStr::new("ssh"), OsStr::new("-T")];
    argv.extend(target.ssh_args.iter().map(OsStr::new));
    argv.extend([OsStr::new(&target.host), OsStr::new("sh"), OsStr::new("-s")]);
    let flags = gio::SubprocessFlags::STDIN_PIPE
        | gio::SubprocessFlags::STDOUT_PIPE
        | gio::SubprocessFlags::STDERR_PIPE;
    let child = gio::Subprocess::newv(&argv, flags).map_err(|e| e.to_string())?;
    let input = glib::Bytes::from_owned(format!("{SESSION_ENV}{script}\n"));
    let (out, err) = child
        .communicate_future(Some(&input))
        .await
        .map_err(|e| e.to_string())?;
    if !child.is_successful() {
        let err = err.map(|e| String::from_utf8_lossy(&e).trim().to_string());
        return Err(err
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "the command failed".into()));
    }
    Ok(out.unwrap_or_else(|| glib::Bytes::from_static(b"")))
}

extern "C" {
    /// The NSWindow behind a GTK macOS surface (GTK 4.8 and later).
    fn gdk_macos_surface_get_native_window(surface: *mut c_void) -> *mut c_void;
}

fn ns_window(window: &gtk::Window) -> Option<Retained<NSWindow>> {
    let surface = window.surface()?;
    // SAFETY: on the macOS backend every GdkSurface is a GdkMacosSurface;
    // GTK returns its NSWindow, or NULL.
    let ptr = unsafe { gdk_macos_surface_get_native_window(surface.as_ptr().cast()) };
    // SAFETY: a non-null result is a live NSWindow that the surface owns;
    // retaining it keeps it valid while we use it.
    unsafe { Retained::retain(ptr.cast::<NSWindow>()) }
}

/// Put the panel just under the item, on the item's screen.
fn place(m: &Menu) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(item_window) = m.item.button(mtm).and_then(|b| b.window()) else {
        return;
    };
    let Some(panel) = ns_window(&m.panel) else {
        return;
    };
    let anchor = item_window.frame();
    let width = panel.frame().size.width;
    let mut x = anchor.origin.x + (anchor.size.width - width) / 2.0;
    if let Some(screen) = item_window.screen() {
        let area = screen.visibleFrame();
        x = x
            .min(area.origin.x + area.size.width - width - 8.0)
            .max(area.origin.x + 8.0);
    }
    panel.setFrameTopLeftPoint(NSPoint::new(x, anchor.origin.y - 4.0));
}
