// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Native platform UI for the windowed runner (KE-0408): the built-in Graphics
//! menu and graphics-settings persistence.
//!
//! Only the windowed runner ([`app`](crate::app)) uses this; the headless driver
//! and every test stay away from it, so no test ever touches the menu bar or the
//! user's stored preferences. The behaviour of the menu (layout, checkmarks,
//! what an item does) lives in the platform-neutral
//! [`settings_menu`](crate::settings_menu) model. This module only renders it
//! natively.
//!
//! macOS is the one implementation today (`NSMenu` in the menu bar plus
//! `NSUserDefaults`, via `objc2`). Other targets get inert stand-ins, so the
//! runner compiles unchanged. The iOS layer (Phase 3) will render the same model
//! as a `UIMenu`.

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub(crate) use macos::{load_settings, save_settings, SettingsMenu};

#[cfg(not(target_os = "macos"))]
pub(crate) use fallback::{load_settings, save_settings, SettingsMenu};

/// Inert stand-ins for targets without a native implementation.
#[cfg(not(target_os = "macos"))]
mod fallback {
    use winit::event_loop::EventLoopProxy;

    use crate::app::EngineEvent;
    use crate::graphics::GraphicsSettings;

    /// No native menu on this target.
    pub(crate) struct SettingsMenu;

    impl SettingsMenu {
        /// Nothing to install.
        pub(crate) fn install(_proxy: EventLoopProxy<EngineEvent>) -> Option<Self> {
            None
        }

        /// Nothing to refresh.
        pub(crate) fn sync(&self, _settings: &GraphicsSettings, _fullscreen: bool) {}
    }

    /// Nothing stored on this target.
    pub(crate) fn load_settings() -> Option<GraphicsSettings> {
        None
    }

    /// Nothing stored on this target.
    pub(crate) fn save_settings(_settings: &GraphicsSettings) {}
}
