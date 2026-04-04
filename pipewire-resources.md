# PipeWire Resources

- [PipeWire docs](https://docs.pipewire.org) — official C API reference, SPA POD/param system
- [pipewire-rs docs](https://docs.rs/pipewire) — Rust bindings API reference
- [pipewire-rs examples](https://gitlab.freedesktop.org/pipewire/pipewire-rs/-/tree/main/pipewire/examples) — `video-capture` example is closest to what this project does
- [another, probably better example](https://github.com/bilelmoussaoui/ashpd/blob/bilelmoussaoui/restructure/examples/screen_cast_pw.rs)
- [PipeWire wiki](https://gitlab.freedesktop.org/pipewire/pipewire/-/wikis/home) — architecture, nodes, graphs, sessions

## Relevant for this project

- [SPA params/format](https://docs.pipewire.org/group__spa__param.html) — explains the `object!` / POD negotiation system
- [pw_stream](https://docs.pipewire.org/group__pw__stream.html) — stream connect/lifecycle used in `src/capture/linux.rs`
