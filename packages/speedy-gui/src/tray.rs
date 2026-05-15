//! System tray integration.
//!
//! Windows + macOS: full menu with Show / Restart daemon / Quit. The status
//! line in the menu reflects whether the daemon is reachable, plus the icon
//! tint shifts (green vs red). On Linux we attempt to create the tray but
//! tolerate failure silently — most distros require AppIndicator/GTK init
//! we don't want to take on here.

use std::sync::atomic::{AtomicBool, Ordering};

use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    Restart,
    Quit,
}

pub struct TrayHandle {
    _tray: TrayIcon,
    status_item: MenuItem,
    show_id: MenuId,
    restart_id: MenuId,
    quit_id: MenuId,
    last_alive: AtomicBool,
}

impl TrayHandle {
    pub fn try_new() -> Option<Self> {
        let menu = Menu::new();
        let status_item = MenuItem::new("Daemon: …", false, None);
        let show = MenuItem::new("Open Speedy", true, None);
        let restart = MenuItem::new("Restart daemon", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let sep = PredefinedMenuItem::separator();
        let sep2 = PredefinedMenuItem::separator();

        menu.append(&status_item).ok()?;
        menu.append(&sep).ok()?;
        menu.append(&show).ok()?;
        menu.append(&restart).ok()?;
        menu.append(&sep2).ok()?;
        menu.append(&quit).ok()?;

        let icon = build_icon(false);
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Speedy")
            .with_icon(icon)
            .build()
            .ok()?;

        Some(Self {
            _tray: tray,
            show_id: show.id().clone(),
            restart_id: restart.id().clone(),
            quit_id: quit.id().clone(),
            status_item,
            last_alive: AtomicBool::new(false),
        })
    }

    /// Update the status line + icon to reflect daemon reachability. Idempotent.
    pub fn set_alive(&self, alive: bool) {
        let prev = self.last_alive.swap(alive, Ordering::Relaxed);
        if prev == alive {
            return;
        }
        let text = if alive { "Daemon: ● alive" } else { "Daemon: ● down" };
        self.status_item.set_text(text);
        let _ = self._tray.set_icon(Some(build_icon(alive)));
    }

    /// Drain the tray's menu-event channel and translate to TrayActions.
    pub fn poll_actions(&self) -> Vec<TrayAction> {
        let mut out = Vec::new();
        let rx = tray_icon::menu::MenuEvent::receiver();
        while let Ok(ev) = rx.try_recv() {
            if ev.id == self.show_id {
                out.push(TrayAction::Show);
            } else if ev.id == self.restart_id {
                out.push(TrayAction::Restart);
            } else if ev.id == self.quit_id {
                out.push(TrayAction::Quit);
            }
        }
        out
    }
}

fn build_icon(alive: bool) -> Icon {
    // 16x16 RGBA. Filled disk in either green (alive) or red (down). Pixels
    // outside the disk are fully transparent so the tray background shows
    // through.
    let w = 16u32;
    let h = 16u32;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let (cr, cg, cb) = if alive { (60u8, 200, 90) } else { (220u8, 70, 70) };
    let center = 7.5f32;
    let radius_sq = 7.0f32 * 7.0f32;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist_sq = dx * dx + dy * dy;
            let idx = ((y * w + x) * 4) as usize;
            if dist_sq <= radius_sq {
                rgba[idx] = cr;
                rgba[idx + 1] = cg;
                rgba[idx + 2] = cb;
                rgba[idx + 3] = 255;
            }
        }
    }
    // Unwrap: 16x16 RGBA is always a valid icon shape.
    Icon::from_rgba(rgba, w, h).expect("build 16x16 tray icon")
}
