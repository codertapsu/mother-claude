//! The tray icon of a run started at sign-in (`autostart`, plan §5.13):
//! Mother Claude runs with no window and serves as usual; *Open Mother
//! Claude* (or a click on the icon, on Windows) shows the window, closing
//! the window hides it again, *Quit Mother Claude* ends the app. On a Mac
//! the icon is in the menu bar, and the app keeps out of the Dock while its
//! window is hidden.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, Window};

const OPEN: &str = "open";
const QUIT: &str = "quit";

/// Puts the icon in the tray (the app's own icon).
pub fn build<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, OPEN, "Open Mother Claude", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit Mother Claude", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Mother Claude")
        .menu(&menu)
        // A Mac shows a menu-bar item's menu on a click; on Windows a click
        // opens the window and the menu is on the right button.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN => show_main(app),
            QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if !cfg!(target_os = "macos") {
                    show_main(tray.app_handle());
                }
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Shows the dashboard's window, in front.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Hides the window instead of closing it: the app goes on in the tray.
pub fn hide_main<R: Runtime>(window: &Window<R>) {
    let _ = window.hide();
    #[cfg(target_os = "macos")]
    let _ = window
        .app_handle()
        .set_activation_policy(tauri::ActivationPolicy::Accessory);
}
