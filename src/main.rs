#![feature(async_drop)]
mod hue;
mod settings;
mod utils;

use std::{pin::pin, time::Duration};

use anyhow::anyhow;
use dotenvy::dotenv;
use rustls::crypto::aws_lc_rs::default_provider;
use scap::{
    capturer::Capturer,
    frame::{BGRxFrame, Frame},
};
use tokio::{select, time::interval};

use crate::{
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

    // Check if the platform is supported
    if !scap::is_supported() {
        return Err(anyhow!("❌ Platform not supported"));
    }

    // Check if we have permission to capture screen
    // If we don't, request it.
    if !scap::has_permission() && !scap::request_permission() {
        return Err(anyhow!("❌ Permission denied"));
    }

    let options = scap::capturer::Options {
        fps: 60,
        output_resolution: scap::capturer::Resolution::_480p,
        ..Default::default()
    };

    let mut capturer = Capturer::build(options)?;

    let mut shutdown = pin!(graceful_shutdown_signal());
    let mut ticker = interval(Duration::from_millis(20));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut hue_entertainment = Hue::new(settings)?.start_entertainment().await?;
    capturer.start_capture();

    let mut loop_body = async || {
        let mut frame = None;
        while let Ok(inner_frame) = capturer.get_next_frame() {
            frame = Some(inner_frame);
        }

        let Some(Frame::BGRx(frame)) = frame else {
            return;
        };

        let middle_pixel_index: usize =
            (4 * (frame.width * (frame.height / 2) + (frame.width / 2))) as usize;

        let [b, g, r]: &[u8; 3] = &frame.data[middle_pixel_index..middle_pixel_index + 3]
            .try_into()
            .unwrap();

        let r: u16 = *r as u16 * 255;
        let g: u16 = *g as u16 * 255;
        let b: u16 = *b as u16 * 255;

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

    capturer.stop_capture();

    Ok(())
}

// fn col_sum(image: &Vec<u8>, width: i32, skip: usize, take: usize) -> [usize; 3] {
//     image
//         .chunks(4 * width as usize)
//         .map(|row| row.chunks(4).skip(skip).take(take))
//         .flatten()
//         .fold([0, 0, 0], |mut acc, dat| {
//             acc[0] += dat[2] as usize;
//             acc[1] += dat[1] as usize;
//             acc[2] += dat[0] as usize;
//             acc
//         })
// }
