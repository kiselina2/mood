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
        let size = 32usize;
        let mut data = vec![0u8; size * size * 4];
        for pixel in data.chunks_exact_mut(4) {
            pixel[0] = 255; // R
            pixel[1] = 160; // G
            pixel[2] = 50;  // B  (warm amber)
            pixel[3] = 255; // A
        }
        tray_icon::Icon::from_rgba(data, size as u32, size as u32)?
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
