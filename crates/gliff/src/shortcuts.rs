//! Command shortcuts on macOS. The system (Command-Tab) and AppKit
//! (Command-Return) act on them before GTK sees the key, and GTK's macOS
//! backend does not implement `inhibit_system_shortcuts`. While the picture
//! has the keyboard, an event tap takes every key event with Command held
//! and hands it to the session instead. Command itself still arrives
//! through GTK.

use std::sync::atomic::{AtomicBool, Ordering};

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use crate::keycode;

/// True while the picture has the keyboard.
static CAPTURING: AtomicBool = AtomicBool::new(false);

pub fn set_capturing(on: bool) {
    CAPTURING.store(on, Ordering::Relaxed);
}

/// Start the tap on its own thread. It yields `(evdev code, pressed)` for
/// each key it takes. The tap needs the Accessibility permission; without it
/// Command shortcuts stay local.
pub fn spawn() -> UnboundedReceiver<(u32, bool)> {
    let (tx, rx) = unbounded_channel();
    let run = move || {
        let tap = CGEventTap::with_enabled(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            vec![CGEventType::KeyDown, CGEventType::KeyUp],
            |_, kind, event| {
                if !CAPTURING.load(Ordering::Relaxed)
                    || !event.get_flags().contains(CGEventFlags::CGEventFlagCommand)
                {
                    return CallbackResult::Keep;
                }
                let mac = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                let Some(code) = keycode::evdev(mac as u32) else {
                    return CallbackResult::Keep;
                };
                match tx.send((code, matches!(kind, CGEventType::KeyDown))) {
                    Ok(()) => CallbackResult::Drop,
                    Err(_) => CallbackResult::Keep,
                }
            },
            CFRunLoop::run_current,
        );
        if tap.is_err() {
            tracing::warn!(
                "Command shortcuts stay local: allow gliff (or the terminal that \
                 runs it) in System Settings > Privacy & Security > Accessibility"
            );
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("gliff-shortcuts".into())
        .spawn(run)
    {
        tracing::warn!(error = %e, "cannot start the shortcut tap");
    }
    rx
}
