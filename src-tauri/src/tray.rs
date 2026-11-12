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

/// The menu's two items, kept so their words follow the window's language.
struct Items<R: Runtime> {
    open: MenuItem<R>,
    quit: MenuItem<R>,
}

/// The menu's words in `lang`, the language the desktop's window shows
/// (English for one this menu does not speak).
fn labels(lang: &str) -> (&'static str, &'static str) {
    match lang {
        "vi" => ("Mở Mother Claude", "Thoát Mother Claude"),
        _ => ("Open Mother Claude", "Quit Mother Claude"),
    }
}

/// The desktop's window says which language it shows (`I18nService`):
/// the menu speaks it. Only the window can tell — the choice is kept in its
/// own storage — so it does, through `invoke`, at its start and at each
/// switch; until then (a run started at sign-in, its window still
/// loading) the menu is in English. No tray, nothing to do.
#[tauri::command]
pub fn tray_language<R: Runtime>(app: AppHandle<R>, lang: String) {
    let Some(items) = app.try_state::<Items<R>>() else {
        return;
    };
    let (open, quit) = labels(&lang);
    let _ = items.open.set_text(open);
    let _ = items.quit.set_text(quit);
}

/// Puts the icon in the tray (the app's own icon).
pub fn build<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let (open, quit) = labels("en");
    let open = MenuItem::with_id(app, OPEN, open, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    app.manage(Items {
        open: open.clone(),
        quit: quit.clone(),
    });
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

#[cfg(test)]
mod tests {
    use super::labels;

    #[test]
    fn the_menu_speaks_the_window_s_language_else_english() {
        assert_eq!(labels("vi"), ("Mở Mother Claude", "Thoát Mother Claude"));
        assert_eq!(labels("en"), ("Open Mother Claude", "Quit Mother Claude"));
        assert_eq!(labels("xx"), labels("en"));
    }
}
