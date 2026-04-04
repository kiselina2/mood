use std::sync::{Arc, Condvar, Mutex};

use anyhow::anyhow;
use ashpd::{
    desktop::{
        PersistMode,
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    },
    enumflags2::BitFlags,
};
use pipewire as pw;
use pw::spa;

use crate::capture::Frame;

struct Terminate;

struct PwUserData {
    format: spa::param::video::VideoInfoRaw,
    state: Arc<(Mutex<Option<Frame>>, Condvar)>,
    ready_tx: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
}

pub struct ScreenCapture {
    state: Arc<(Mutex<Option<Frame>>, Condvar)>,
    quit_tx: Option<pw::channel::Sender<Terminate>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ScreenCapture {
    pub async fn new() -> anyhow::Result<Self> {
        let proxy = Screencast::new().await?;
        let session = proxy.create_session(Default::default()).await?;
        proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(CursorMode::Hidden)
                    .set_sources(BitFlags::from(SourceType::Monitor))
                    .set_multiple(false)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await?;
        let response = proxy
            .start(&session, None, Default::default())
            .await?
            .response()?;
        let node_id = response
            .streams()
            .first()
            .ok_or_else(|| anyhow!("no stream selected"))?
            .pipe_wire_node_id();
        let fd = proxy
            .open_pipe_wire_remote(&session, Default::default())
            .await?;

        let state = Arc::new((Mutex::new(None::<Frame>), Condvar::new()));
        let (quit_tx, quit_rx) = pw::channel::channel::<Terminate>();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();

        let state_clone = Arc::clone(&state);
        let thread = std::thread::spawn(move || {
            run_pipewire(node_id, fd, state_clone, quit_rx, ready_tx);
        });

        ready_rx
            .await
            .map_err(|_| anyhow!("PipeWire thread exited before stream was ready"))?
            .map_err(|e| anyhow!("PipeWire stream error: {e}"))?;

        Ok(Self {
            state,
            quit_tx: Some(quit_tx),
            thread: Some(thread),
        })
    }

    /// Returns the latest captured frame. Blocks on the first call until
    /// the PipeWire stream delivers its first frame. Subsequent calls
    /// return immediately, repeating the previous frame if no new one arrived.
    pub fn get_latest_frame(&self) -> Frame {
        let (lock, condvar) = &*self.state;
        let guard = condvar
            .wait_while(lock.lock().unwrap(), |f| f.is_none())
            .unwrap();
        guard.as_ref().unwrap().clone()
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        if let Some(tx) = self.quit_tx.take() {
            let _ = tx.send(Terminate);
        }
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

fn run_pipewire(
    node_id: u32,
    fd: std::os::fd::OwnedFd,
    state: Arc<(Mutex<Option<Frame>>, Condvar)>,
    quit_rx: pw::channel::Receiver<Terminate>,
    ready_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
) {
    pw::init();

    let mainloop =
        pw::main_loop::MainLoopRc::new(None).expect("failed to create PipeWire main loop");
    let context =
        pw::context::ContextRc::new(&mainloop, None).expect("failed to create PipeWire context");
    let core = context
        .connect_fd_rc(fd, None)
        .expect("failed to connect to PipeWire");

    let _quit = quit_rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    let stream = pw::stream::StreamRc::new(
        core,
        "mood-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .expect("failed to create PipeWire stream");

    let _listener = stream
        .add_local_listener_with_user_data(PwUserData {
            format: Default::default(),
            state,
            ready_tx: Some(ready_tx),
        })
        .state_changed(|_, user_data, _old, new| match new {
            pw::stream::StreamState::Paused => {
                if let Some(tx) = user_data.ready_tx.take() {
                    let _ = tx.send(Ok(()));
                }
            }
            pw::stream::StreamState::Error(msg) => {
                if let Some(tx) = user_data.ready_tx.take() {
                    let _ = tx.send(Err(msg.to_string()));
                }
            }
            _ => {}
        })
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param)
            else {
                return;
            };
            if media_type != spa::param::format::MediaType::Video
                || media_subtype != spa::param::format::MediaSubtype::Raw
            {
                return;
            }
            user_data
                .format
                .parse(param)
                .expect("failed to parse video format");
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let Some(raw) = datas[0].data() else { return };

            let width = user_data.format.size().width;
            let height = user_data.format.size().height;
            if width == 0 || height == 0 {
                return;
            }

            let mut data = raw.to_vec();

            // Normalize to RGBA: swap R and B channels for BGR* formats
            if matches!(
                user_data.format.format(),
                spa::param::video::VideoFormat::BGRx | spa::param::video::VideoFormat::BGRA
            ) {
                for pixel in data.chunks_mut(4) {
                    pixel.swap(0, 2);
                }
            }

            let frame = Frame {
                data: Arc::from(data),
                width,
                height,
            };

            let (lock, condvar) = &*user_data.state;
            *lock.lock().unwrap() = Some(frame);
            condvar.notify_one();
        })
        .register()
        .expect("failed to register stream listener");

    let obj = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        pw::spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        pw::spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRA,
        ),
        pw::spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            pw::spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            pw::spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            pw::spa::utils::Rectangle {
                width: 4096,
                height: 4096
            }
        ),
        pw::spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            pw::spa::utils::Fraction { num: 50, denom: 1 },
            pw::spa::utils::Fraction { num: 0, denom: 1 },
            pw::spa::utils::Fraction { num: 50, denom: 1 }
        ),
    );

    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .unwrap()
    .0
    .into_inner();

    let mut params = [spa::pod::Pod::from_bytes(&values).unwrap()];

    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .expect("failed to connect PipeWire stream");

    mainloop.run();
}
