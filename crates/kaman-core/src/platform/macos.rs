// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! macOS: the Graphics menu as an `NSMenu` in the menu bar, and settings
//! persistence in `NSUserDefaults` (KE-0408).
//!
//! # How a click gets back into the engine loop
//!
//! 1. Every settings item targets one `KamanMenuTarget`, a tiny `NSObject`
//!    subclass declared with `objc2`, and uses one action,
//!    `kamanGraphicsItemSelected:`. The item's integer `tag` encodes which
//!    [`MenuCommand`] it is (see [`MenuCommand::tag`]).
//! 2. AppKit calls that action on the main thread, from its own run loop, never
//!    in the middle of an engine frame. The target decodes the tag and posts an
//!    [`EngineEvent`] through winit's `EventLoopProxy`, which wakes the loop.
//! 3. The runner's `user_event` handler turns the command into a graphics-settings
//!    request, the same queue [`EngineCtx::set_graphics_settings`](crate::EngineCtx::set_graphics_settings)
//!    uses.
//!
//! There are no globals and no `static mut`: the proxy lives in the target's
//! instance variables, and the runner owns the target (an `NSMenuItem` holds its
//! target weakly).
//!
//! # Coexisting with winit's menu
//!
//! winit builds the standard application menu (About, Hide, Quit, …) before the
//! first `resumed`. The Graphics menu is **appended** to that menu bar rather
//! than replacing it, so the standard items keep working.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{declare_class, msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu,
    NSMenuItem,
};
use objc2_foundation::{MainThreadMarker, NSString, NSUserDefaults};
use winit::event_loop::EventLoopProxy;

use crate::app::EngineEvent;
use crate::graphics::GraphicsSettings;
use crate::settings_menu::{submenus, MenuCommand, ACTIONS, MENU_TITLE};

/// Instance variables of [`KamanMenuTarget`]: where to post clicks.
pub(crate) struct TargetIvars {
    proxy: EventLoopProxy<EngineEvent>,
}

declare_class!(
    /// The action target shared by every Graphics menu item.
    struct KamanMenuTarget;

    unsafe impl ClassType for KamanMenuTarget {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "KamanMenuTarget";
    }

    impl DeclaredClass for KamanMenuTarget {
        type Ivars = TargetIvars;
    }

    unsafe impl KamanMenuTarget {
        #[method(kamanGraphicsItemSelected:)]
        fn item_selected(&self, sender: &NSMenuItem) {
            // SAFETY: reading an `NSMenuItem`'s tag on the main thread (this
            // class is main-thread-only).
            let tag = unsafe { sender.tag() };
            if let Some(command) = MenuCommand::from_tag(tag) {
                // Fails only if the event loop has already exited, and then
                // there is nothing left to apply the command to.
                let _ = self.ivars().proxy.send_event(EngineEvent::Menu(command));
            }
        }
    }

    unsafe impl NSObjectProtocol for KamanMenuTarget {}
);

impl KamanMenuTarget {
    fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<EngineEvent>) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(TargetIvars { proxy });
        // SAFETY: `NSObject`'s designated initializer, called once on a fresh
        // allocation whose ivars are set.
        unsafe { msg_send_id![super(this), init] }
    }
}

/// The installed Graphics menu: the native items, and the target they call.
pub(crate) struct SettingsMenu {
    /// Kept alive here: menu items reference their target weakly.
    _target: Retained<KamanMenuTarget>,
    /// Every item that stands for a [`MenuCommand`], for [`sync`](Self::sync).
    items: Vec<(Retained<NSMenuItem>, MenuCommand)>,
}

impl SettingsMenu {
    /// Build the Graphics menu and append it to the application's menu bar.
    ///
    /// Returns `None` off the main thread (never the case in the runner's
    /// `resumed`, which winit calls on the main thread).
    pub(crate) fn install(proxy: EventLoopProxy<EngineEvent>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let target = KamanMenuTarget::new(mtm, proxy);
        let action = sel!(kamanGraphicsItemSelected:);
        let target_obj: &AnyObject = &target;

        let make_item = |command: MenuCommand| -> Retained<NSMenuItem> {
            let (key, mask) = match command {
                // ⌃⌘F, the system's standard full-screen shortcut.
                MenuCommand::ToggleFullScreen => (
                    "f",
                    Some(
                        NSEventModifierFlags::NSEventModifierFlagControl
                            | NSEventModifierFlags::NSEventModifierFlagCommand,
                    ),
                ),
                _ => ("", None),
            };
            // SAFETY: plain AppKit object construction and configuration on the
            // main thread; the selector names the method `KamanMenuTarget`
            // implements, and the target outlives the item's use of it (both are
            // owned by the returned `SettingsMenu`).
            unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &NSString::from_str(command.label(false)),
                    Some(action),
                    &NSString::from_str(key),
                );
                if let Some(mask) = mask {
                    item.setKeyEquivalentModifierMask(mask);
                }
                item.setTarget(Some(target_obj));
                item.setTag(command.tag());
                item
            }
        };

        let titled_menu = |title: &str| -> Retained<NSMenu> {
            // SAFETY: main-thread AppKit construction. Items are enabled/disabled
            // explicitly by `sync`, so automatic enabling is turned off.
            unsafe {
                let menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(title));
                menu.setAutoenablesItems(false);
                menu
            }
        };
        let submenu_item = |title: &str, submenu: &NSMenu| -> Retained<NSMenuItem> {
            // SAFETY: as above; a parent item with no action, only a submenu.
            unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &NSString::from_str(title),
                    None,
                    &NSString::from_str(""),
                );
                item.setSubmenu(Some(submenu));
                item
            }
        };

        let graphics = titled_menu(MENU_TITLE);
        let mut items = Vec::new();
        for sub in submenus() {
            let menu = titled_menu(sub.title);
            for command in sub.items {
                let item = make_item(command);
                menu.addItem(&item);
                items.push((item, command));
            }
            graphics.addItem(&submenu_item(sub.title, &menu));
        }
        graphics.addItem(&NSMenuItem::separatorItem(mtm));
        for command in ACTIONS {
            let item = make_item(command);
            graphics.addItem(&item);
            items.push((item, command));
        }

        let app = NSApplication::sharedApplication(mtm);
        // SAFETY: main-thread access to the shared application's menu bar.
        let menu_bar = unsafe { app.mainMenu() }.unwrap_or_else(|| {
            // Without winit's default menu there is no bar yet: make one.
            let bar = NSMenu::new(mtm);
            app.setMainMenu(Some(&bar));
            bar
        });
        menu_bar.addItem(&submenu_item(MENU_TITLE, &graphics));

        Some(Self {
            _target: target,
            items,
        })
    }

    /// Refresh checkmarks, enabled states and the full-screen title for
    /// `settings` and the window's full-screen state.
    pub(crate) fn sync(&self, settings: &GraphicsSettings, fullscreen: bool) {
        for (item, command) in &self.items {
            // SAFETY: main-thread configuration of items this menu owns (the
            // runner calls `sync` only from winit callbacks on the main thread).
            unsafe {
                item.setState(if command.is_checked(settings) {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                item.setEnabled(command.is_enabled(settings));
                if *command == MenuCommand::ToggleFullScreen {
                    item.setTitle(&NSString::from_str(command.label(fullscreen)));
                }
            }
        }
    }
}

/// `NSUserDefaults` keys for the four settings, in
/// [`GraphicsSettings::to_codes`] order.
const DEFAULTS_KEYS: [&str; 4] = [
    "KamanEngine.graphics.shadows",
    "KamanEngine.graphics.shadowDistance",
    "KamanEngine.graphics.drawDistance",
    "KamanEngine.graphics.renderScale",
];

/// Read the settings saved by a previous launch, or `None` if none were saved.
///
/// Stored values are small integer codes (see [`GraphicsSettings::to_codes`]);
/// an unknown or missing code decodes to that setting's default.
pub(crate) fn load_settings() -> Option<GraphicsSettings> {
    // SAFETY: `NSUserDefaults` is thread-safe; reading integers by key.
    let codes = unsafe {
        let defaults = NSUserDefaults::standardUserDefaults();
        DEFAULTS_KEYS.map(|key| defaults.integerForKey(&NSString::from_str(key)) as i64)
    };
    (codes != [0; 4]).then(|| GraphicsSettings::from_codes(codes))
}

/// Save `settings` for the next launch.
pub(crate) fn save_settings(settings: &GraphicsSettings) {
    // SAFETY: `NSUserDefaults` is thread-safe; writing integers by key.
    unsafe {
        let defaults = NSUserDefaults::standardUserDefaults();
        for (key, code) in DEFAULTS_KEYS.iter().zip(settings.to_codes()) {
            defaults.setInteger_forKey(code as isize, &NSString::from_str(key));
        }
    }
}
