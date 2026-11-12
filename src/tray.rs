//! System tray icon and its context menu.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::{Key, Lang};
use crate::platform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    SyncAll,
    TogglePause,
    Settings,
    Quit,
}

pub struct TrayState {
    tray: TrayIcon,
    show_item: MenuItem,
    sync_item: MenuItem,
    pause_item: MenuItem,
    settings_item: MenuItem,
    quit_item: MenuItem,
}

impl TrayState {
    pub fn new(lang: Lang, paused: bool, tooltip: &str) -> Option<TrayState> {
        let menu = Menu::new();
        let show_item = MenuItem::with_id("show", Key::TrayShow.tr(lang), true, None);
        let sync_item = MenuItem::with_id("sync_all", Key::TraySyncAll.tr(lang), true, None);
        let pause_item = MenuItem::with_id(
            "toggle_pause",
            if paused {
                Key::TrayResumeAll.tr(lang)
            } else {
                Key::TrayPauseAll.tr(lang)
            },
            true,
            None,
        );
        let settings_item =
            MenuItem::with_id("settings", Key::TraySettings.tr(lang), true, None);
        let quit_item = MenuItem::with_id("quit", Key::TrayExit.tr(lang), true, None);

        menu.append(&show_item).ok()?;
        menu.append(&sync_item).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&pause_item).ok()?;
        menu.append(&settings_item).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&quit_item).ok()?;

        let icon = tray_icon::Icon::from_rgba(platform::icon_rgba(64), 64, 64).ok()?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(tooltip)
            .with_icon(icon)
            .build()
            .ok()?;

        Some(TrayState {
            tray,
            show_item,
            sync_item,
            pause_item,
            settings_item,
            quit_item,
        })
    }

    pub fn set_language(&self, lang: Lang, paused: bool) {
        self.show_item.set_text(Key::TrayShow.tr(lang));
        self.sync_item.set_text(Key::TraySyncAll.tr(lang));
        self.pause_item.set_text(if paused {
            Key::TrayResumeAll.tr(lang)
        } else {
            Key::TrayPauseAll.tr(lang)
        });
        self.settings_item.set_text(Key::TraySettings.tr(lang));
        self.quit_item.set_text(Key::TrayExit.tr(lang));
    }

    pub fn set_paused(&self, lang: Lang, paused: bool) {
        self.pause_item.set_text(if paused {
            Key::TrayResumeAll.tr(lang)
        } else {
            Key::TrayPauseAll.tr(lang)
        });
    }

    pub fn set_tooltip(&self, text: &str) {
        let _ = self.tray.set_tooltip(Some(text));
    }

    pub fn poll(&self) -> Vec<TrayCommand> {
        let mut commands = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            match event.id.0.as_str() {
                "show" => commands.push(TrayCommand::Show),
                "sync_all" => commands.push(TrayCommand::SyncAll),
                "toggle_pause" => commands.push(TrayCommand::TogglePause),
                "settings" => commands.push(TrayCommand::Settings),
                "quit" => commands.push(TrayCommand::Quit),
                _ => {}
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                commands.push(TrayCommand::Show);
            }
        }
        commands
    }
}
