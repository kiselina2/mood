#![feature(async_drop)]
mod hue;
mod settings;
mod utils;

use std::{pin::pin, time::Duration};

use dotenvy::dotenv;
use rustls::crypto::aws_lc_rs::default_provider;
use tokio::{select, time::interval};

use crate::{
    hue::{Color, Hue},
    settings::AppSettings,
    utils::graceful_shutdown_signal,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    let mut settings = AppSettings::load()?;

    if std::env::args().any(|a| a == "--setup") {
        settings.run_setup()?;
        return Ok(());
    }

    default_provider()
        .install_default()
        .expect("couldn't install aws_lc_rs default crypto provider");

    let (r, g, b) = (100u16 * 255, 255u16 * 255, 10u16 * 255);

    let mut shutdown = pin!(graceful_shutdown_signal());
    let mut ticker = interval(Duration::from_millis(20));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut hue_entertainment = Hue::new(settings)?.start_entertainment().await?;

    loop {
        select! {
            _ = &mut shutdown => { break }
            _ = ticker.tick() => {
                if let Err(e) = hue_entertainment.send_colors(&[Color::new(r, g, b), Color::new(r, g, b)]).await {
                    dbg_print!("{e}");
                }
            }
        }
    }

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
