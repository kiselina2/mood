use std::time::Duration;

use anyhow::anyhow;
use tokio::sync::oneshot;
use tray_icon::{
    TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};

pub fn run(shutdown_tx: oneshot::Sender<()>) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    gtk::init().map_err(|e| anyhow!("GTK init failed: {e}"))?;

    let menu = Menu::new();
    let quit_item = MenuItem::new("Quit Mood", true, None);
    menu.append(&quit_item)?;

    let icon = {
        let png_bytes = include_bytes!("../icon.png");
        let img = image::load_from_memory(png_bytes)
            .map_err(|e| anyhow!("failed to load tray icon: {e}"))?
            .into_rgba8();
        let (w, h) = img.dimensions();
        tray_icon::Icon::from_rgba(img.into_raw(), w, h)?
    };

    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Mood")
        .with_icon(icon)
        .build()?;

    let menu_events = MenuEvent::receiver();
    let quit_id = quit_item.id().clone();
    let mut shutdown_tx = Some(shutdown_tx);

    loop {
        #[cfg(target_os = "linux")]
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }

        if let Ok(event) = menu_events.try_recv() {
            if event.id == quit_id {
                if let Some(tx) = shutdown_tx.take() {
                    let _ = tx.send(());
                }
                break;
            }
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}
