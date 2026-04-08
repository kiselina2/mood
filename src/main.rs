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
    hue::{Color, ColorBuffer, Hue},
    settings::AppSettings,
    utils::graceful_shutdown_signal,
};

const FPS: u32 = 60;
const FRAMES_SENT_PER_SECOND: u32 = 50;
const COLOR_SMOOTHING_IN_MILLIS: u32 = 100;
const COLOR_BUFFER_LENGTH: usize =
    ((FRAMES_SENT_PER_SECOND * COLOR_SMOOTHING_IN_MILLIS) / 1000) as usize;

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
        fps: FPS,
        output_resolution: scap::capturer::Resolution::_480p,
        ..Default::default()
    };

    let mut capturer = Capturer::build(options)?;

    let mut shutdown = pin!(graceful_shutdown_signal());
    let mut ticker = interval(Duration::from_millis(1000 / FRAMES_SENT_PER_SECOND as u64));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut hue_entertainment = Hue::new(settings)?.start_entertainment().await?;
    capturer.start_capture();

    let mut color_buffer1 = ColorBuffer::<COLOR_BUFFER_LENGTH>::new();
    let mut color_buffer2 = ColorBuffer::<COLOR_BUFFER_LENGTH>::new();

    let mut send_colors =
        async |color_buffer1: &ColorBuffer<COLOR_BUFFER_LENGTH>,
               color_buffer2: &ColorBuffer<COLOR_BUFFER_LENGTH>| {
            let first_avg = color_buffer1.avg();

            // dbg_print!("{:?}", first_avg);

            if let Err(e) = hue_entertainment
                .send_colors(&[first_avg, color_buffer2.avg()])
                .await
            {
                dbg_print!("{e:?}");
            }
        };

    let mut loop_body = async || {
        let mut frame = None;
        while let Ok(inner_frame) = capturer.get_next_frame() {
            frame = Some(inner_frame);
        }

        let Some(Frame::BGRx(frame)) = frame else {
            color_buffer1.debug();
            color_buffer1.dupe_last();
            color_buffer2.dupe_last();
            send_colors(&color_buffer1, &color_buffer2).await;
            return;
        };

        let [color1, color2] = get_average_colors_from_frame(&frame);

        color_buffer1.push(color1);
        color_buffer2.push(color2);

        send_colors(&color_buffer1, &color_buffer2).await
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

fn get_average_colors_from_frame(frame: &BGRxFrame) -> [Color; 2] {
    let width = frame.width as usize;
    let row_bytes = 4 * width;

    let skip_rows = row_bytes * (frame.height as f32 * 0.1) as usize;
    let data = &frame.data[skip_rows..frame.data.len() - skip_rows];

    let left_start = (width as f32 * 0.05) as usize;
    let left_end = (width as f32 * 0.10) as usize;
    let right_start = (width as f32 * 0.90) as usize;
    let right_end = (width as f32 * 0.95) as usize;

    let num_rows = data.len() / row_bytes;
    let count = ((left_end - left_start) * num_rows) as i32;

    let sum_strip = |start: usize, end: usize| -> (i32, i32, i32) {
        data.chunks_exact(row_bytes)
            .fold((0, 0, 0), |(r, g, b), row| {
                row[4 * start..4 * end]
                    .chunks_exact(4)
                    .fold((r, g, b), |(r, g, b), p| {
                        (r + p[2] as i32, g + p[1] as i32, b + p[0] as i32)
                    })
            })
    };

    let (r1, g1, b1) = sum_strip(left_start, left_end);
    let (r2, g2, b2) = sum_strip(right_start, right_end);

    [
        Color {
            r: (r1 / count) as u16 * 257,
            g: (g1 / count) as u16 * 257,
            b: (b1 / count) as u16 * 257,
        },
        Color {
            r: (r2 / count) as u16 * 257,
            g: (g2 / count) as u16 * 257,
            b: (b2 / count) as u16 * 257,
        },
    ]
}
