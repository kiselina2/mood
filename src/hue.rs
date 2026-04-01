use std::{error::Error, future::AsyncDrop, sync::Arc};

use dtls::{
    cipher_suite::CipherSuiteId,
    config::{Config, ExtendedMasterSecretType},
    conn::DTLSConn,
};
use reqwest::{
    Certificate, Client,
    header::{HeaderMap, HeaderValue},
};
use tokio::net::UdpSocket;

use crate::{dbg_print, settings::AppSettings};

pub struct Hue {
    rest_client: Client,
    settings: AppSettings,
}

pub struct HueEntertainment {
    rest_client: Client,
    dtls_connection: DTLSConn,
    settings: AppSettings,
    lamp_snapshots: Vec<LampSnapshot>,
    packet: Vec<u8>,
    header_len: usize,
}

pub struct Color {
    pub r: u16,
    pub g: u16,
    pub b: u16,
}

struct LampSnapshot {
    id: String,
    on: bool,
    brightness: Option<f64>,
    color_xy: Option<(f64, f64)>,
    color_temperature: Option<u32>,
    color_mode: String,
}

impl Color {
    pub fn new(r: u16, g: u16, b: u16) -> Self {
        Color { r, g, b }
    }
}

impl HueEntertainment {
    fn new(
        rest_client: Client,
        dtls_connection: DTLSConn,
        settings: AppSettings,
        lamp_snapshots: Vec<LampSnapshot>,
    ) -> Result<Self, Box<dyn Error>> {
        let [entertainment_config_id] = settings.config.get(["ENTERTAINMENT_CONFIG_ID"])?;

        let mut packet = Vec::with_capacity(30 + entertainment_config_id.len());
        packet.extend_from_slice(b"HueStream"); // header
        packet.extend_from_slice(&[0x02, 0x00]); // version
        packet.push(0x00); // sequence
        packet.extend_from_slice(&[0x00, 0x00]); // reserved
        packet.push(0x00); // color mode: RGB
        packet.push(0x00); // reserved
        packet.extend_from_slice(entertainment_config_id.as_bytes()); // entertainment config id
        let header_len = packet.len();

        Ok(HueEntertainment {
            rest_client,
            dtls_connection,
            settings,
            lamp_snapshots,
            packet,
            header_len,
        })
    }

    pub async fn send_colors(&mut self, colors: &[Color; 2]) -> Result<(), Box<dyn Error>> {
        self.packet.push(0x00); // light id
        self.packet.extend_from_slice(&colors[0].r.to_be_bytes()); // R
        self.packet.extend_from_slice(&colors[0].g.to_be_bytes()); // G
        self.packet.extend_from_slice(&colors[0].b.to_be_bytes()); // B
        self.packet.push(0x01); // light id
        self.packet.extend_from_slice(&colors[1].r.to_be_bytes()); // R
        self.packet.extend_from_slice(&colors[1].g.to_be_bytes()); // G
        self.packet.extend_from_slice(&colors[1].b.to_be_bytes()); // B

        let result = self.dtls_connection.write(&self.packet, None).await;
        self.packet.truncate(self.header_len);
        result?;

        Ok(())
    }
}

impl Hue {
    pub fn new(settings: AppSettings) -> Result<Self, Box<dyn Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "hue-application-key",
            HeaderValue::from_str(&(settings.secrets.get(["APP_KEY"])?[0]))?,
        );

        Ok(Hue {
            rest_client: Client::builder()
                .tls_certs_only([Certificate::from_pem(include_bytes!("../hue_cert.pem"))?])
                .default_headers(headers)
                .danger_accept_invalid_hostnames(true)
                .build()?,
            settings,
        })
    }

    pub async fn start_entertainment(self) -> Result<HueEntertainment, Box<dyn Error>> {
        let [client_key] = self.settings.secrets.get(["CLIENT_KEY"])?;

        let [bridge_ip, bridge_port, app_id, entertainment_config_id] =
            self.settings.config.get([
                "BRIDGE_ADDRESS",
                "BRIDGE_PORT",
                "APP_ID",
                "ENTERTAINMENT_CONFIG_ID",
            ])?;

        let light_ids =
            fetch_light_ids(&self.rest_client, bridge_ip, entertainment_config_id).await?;
        let mut lamp_snapshots = Vec::with_capacity(light_ids.len());
        for id in &light_ids {
            lamp_snapshots.push(fetch_lamp_snapshot(&self.rest_client, bridge_ip, id).await?);
        }

        set_entertainment_state(
            &self.rest_client,
            bridge_ip,
            entertainment_config_id,
            EntertainmentAction::Start,
        )
        .await?;

        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(format!("{bridge_ip}:{bridge_port}")).await?;

        let psk = hex::decode(client_key)?;

        let dtls_config = Config {
            psk: Some(Arc::new(move |_hint: &[u8]| {
                let key = psk.clone();
                Box::pin(async move { Ok(key) })
            })),
            psk_identity_hint: Some(app_id.clone().into_bytes()),
            extended_master_secret: ExtendedMasterSecretType::Require,
            cipher_suites: vec![CipherSuiteId::Tls_Psk_With_Aes_128_Gcm_Sha256],
            ..Default::default()
        };

        let dtls_connection = match DTLSConn::new(Arc::new(socket), dtls_config, true, None).await {
            Ok(conn) => conn,
            Err(e) => {
                if let Err(stop_err) = set_entertainment_state(
                    &self.rest_client,
                    &bridge_ip,
                    &entertainment_config_id,
                    EntertainmentAction::Stop,
                )
                .await
                {
                    dbg_print!("failed to stop entertainment during DTLS cleanup: {stop_err}");
                }
                return Err(e.into());
            }
        };

        Ok(HueEntertainment::new(
            self.rest_client,
            dtls_connection,
            self.settings,
            lamp_snapshots,
        )?)
    }
}

// Required by the AsyncDrop feature: a synchronous Drop impl must exist
// alongside AsyncDrop. Actual cleanup is done in AsyncDrop::drop.
impl Drop for HueEntertainment {
    fn drop(&mut self) {}
}

impl AsyncDrop for HueEntertainment {
    async fn drop(self: std::pin::Pin<&mut Self>) {
        let [bridge_ip, entertainment_config_id] = self
            .settings
            .config
            .get(["BRIDGE_ADDRESS", "ENTERTAINMENT_CONFIG_ID"])
            .unwrap();

        if let Err(e) = set_entertainment_state(
            &self.rest_client,
            &bridge_ip,
            &entertainment_config_id,
            EntertainmentAction::Stop,
        )
        .await
        {
            dbg_print!("failed to stop entertainment: {e}");
        }
        if let Err(e) = self.dtls_connection.close().await {
            dbg_print!("failed to close DTLS connection: {e}");
        }

        for snapshot in &self.lamp_snapshots {
            if let Err(e) = restore_lamp(&self.rest_client, &bridge_ip, snapshot).await {
                dbg_print!("failed to restore lamp {}: {e}", snapshot.id);
            }
        }
    }
}

enum EntertainmentAction {
    Start,
    Stop,
}

impl From<EntertainmentAction> for &'static str {
    fn from(action: EntertainmentAction) -> &'static str {
        match action {
            EntertainmentAction::Start => "start",
            EntertainmentAction::Stop => "stop",
        }
    }
}

async fn set_entertainment_state(
    client: &Client,
    bridge_ip: &str,
    entertainment_config_id: &str,
    action: EntertainmentAction,
) -> Result<(), Box<dyn Error>> {
    let action: &str = action.into();
    let response = client
        .put(format!(
            "https://{bridge_ip}/clip/v2/resource/entertainment_configuration/{entertainment_config_id}"
        ))
        .json(&serde_json::json!({ "action": action }))
        .send()
        .await?;
    if let Err(e) = response.error_for_status_ref() {
        dbg_print!("set_entertainment_state HTTP error: {e}");
    }
    Ok(())
}

async fn fetch_light_ids(
    client: &Client,
    bridge_ip: &str,
    entertainment_config_id: &str,
) -> Result<Vec<String>, Box<dyn Error>> {
    let response = client
        .get(format!(
            "https://{bridge_ip}/clip/v2/resource/entertainment_configuration/{entertainment_config_id}"
        ))
        .send()
        .await?;
    if let Err(e) = response.error_for_status_ref() {
        dbg_print!("fetch_light_ids HTTP error: {e}");
    }
    let response: serde_json::Value = response.json().await?;

    let mut ids = Vec::new();
    if let Some(services) = response["data"][0]["light_services"].as_array() {
        for service in services {
            if let Some(rid) = service["rid"].as_str() {
                ids.push(rid.to_string());
            }
        }
    }

    Ok(ids)
}

async fn fetch_lamp_snapshot(
    client: &Client,
    bridge_ip: &str,
    light_id: &str,
) -> Result<LampSnapshot, Box<dyn Error>> {
    let response = client
        .get(format!(
            "https://{bridge_ip}/clip/v2/resource/light/{light_id}"
        ))
        .send()
        .await?;
    if let Err(e) = response.error_for_status_ref() {
        dbg_print!("fetch_lamp_snapshot HTTP error: {e}");
    }
    let response: serde_json::Value = response.json().await?;

    let data = &response["data"][0];

    let on = data["on"]["on"].as_bool().unwrap_or(false);
    let brightness = data["dimming"]["brightness"].as_f64();
    let color_xy = data["color"]["xy"]["x"].as_f64()
        .zip(data["color"]["xy"]["y"].as_f64());
    let color_temperature = data["color_temperature"]["mirek"]
        .as_u64()
        .map(|v| v as u32);
    let mirek_valid = data["color_temperature"]["mirek_valid"]
        .as_bool()
        .unwrap_or(false);
    let color_mode = if mirek_valid {
        "color_temperature"
    } else {
        "color_xy"
    }
    .to_string();

    Ok(LampSnapshot {
        id: light_id.to_string(),
        on,
        brightness,
        color_xy,
        color_temperature,
        color_mode,
    })
}

async fn restore_lamp(
    client: &Client,
    bridge_ip: &str,
    snapshot: &LampSnapshot,
) -> Result<(), Box<dyn Error>> {
    let mut body = serde_json::json!({
        "on": { "on": snapshot.on }
    });

    if let Some(brightness) = snapshot.brightness {
        body["dimming"] = serde_json::json!({ "brightness": brightness });
    }

    match snapshot.color_mode.as_str() {
        "color_temperature" => {
            if let Some(mirek) = snapshot.color_temperature {
                body["color_temperature"] = serde_json::json!({ "mirek": mirek });
            }
        }
        "color_xy" => {
            if let Some((x, y)) = snapshot.color_xy {
                body["color"] = serde_json::json!({ "xy": { "x": x, "y": y } });
            }
        }
        _ => {}
    }

    let response = client
        .put(format!(
            "https://{bridge_ip}/clip/v2/resource/light/{}",
            snapshot.id
        ))
        .json(&body)
        .send()
        .await?;
    if let Err(e) = response.error_for_status_ref() {
        dbg_print!("restore_lamp HTTP error: {e}");
    }

    Ok(())
}
