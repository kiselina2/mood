#![feature(async_drop)]
mod capture;
mod hue;
mod settings;
mod utils;

use std::{pin::pin, time::Duration};

use dotenvy::dotenv;
use rustls::crypto::aws_lc_rs::default_provider;
use tokio::{select, time::interval};

use crate::{
    capture::ScreenCapture,
    hue::{Color, Hue},
    settings::AppSettings,
    utils::graceful_shutdown_signal,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    default_provider()
        .install_default()
        .expect("couldn't install aws_lc_rs default crypto provider");

    let mut settings = AppSettings::load()?;

    if std::env::args().any(|a| a == "--setup") {
        settings.run_setup()?;
        return Ok(());
    }

    let capture = ScreenCapture::new().await?;

    let mut shutdown = pin!(graceful_shutdown_signal());
    let mut ticker = interval(Duration::from_millis(20));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut hue_entertainment = Hue::new(settings)?.start_entertainment().await?;

    let mut loop_body = async || {
        let (r, g, b) = {
            let frame = capture.get_latest_frame();
            let mid = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
            (
                frame.data[mid] as u16 * 255,
                frame.data[mid + 1] as u16 * 255,
                frame.data[mid + 2] as u16 * 255,
            )
        };

        dbg_print!("{r} {g} {b}");

        let colors = &[Color::new(r, g, b), Color::new(r, g, b)];
        if let Err(e) = hue_entertainment.send_colors(colors).await {
            dbg_print!("{e}");
        }
    };

    loop {
        select! {
            _ = &mut shutdown => { break }
            _ = ticker.tick() => loop_body().await
        }
    }

    Ok(())
}
