//! Shared utilities

/// Prints to stdout in debug builds only. Compiles to nothing in release builds.
#[macro_export]
macro_rules! dbg_print {
    ($($arg:tt)*) => { #[cfg(debug_assertions)] println!($($arg)*); }
}

/// Waits for a graceful shutdown signal from the OS or the tray icon.
///
/// On Unix, resolves on `SIGINT` or `SIGTERM`.
/// On Windows, resolves on `Ctrl+C`, `CTRL_CLOSE_EVENT`, or `CTRL_SHUTDOWN_EVENT`.
/// Also resolves when the tray sends a quit signal via `tray_quit`.
pub async fn graceful_shutdown_signal(tray_quit: tokio::sync::oneshot::Receiver<()>) {
    #[cfg(unix)]
    {
        use tokio::{
            select,
            signal::unix::{SignalKind, signal},
        };
        let mut sigint =
            signal(SignalKind::interrupt()).expect("failed to register SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
        select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
            _ = tray_quit => {}
        }
    }
    #[cfg(windows)]
    {
        use tokio::{select, signal::windows};
        let mut close = windows::ctrl_close().expect("failed to register ctrl_close handler");
        let mut shutdown =
            windows::ctrl_shutdown().expect("failed to register ctrl_shutdown handler");
        select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = close.recv() => {}
            _ = shutdown.recv() => {}
            _ = tray_quit => {}
        }
    }
}
