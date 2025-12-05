# autotube

autotube automatically downloads new videos published by YouTube channels you chose to follow, by periodically checking the channels' RSS feeds for updates and downloading videos published since the last check by handing them off to [`yt-dlp`](https://github.com/yt-dlp/yt-dlp).

:warning: **autotube does not handle authentication!** :warning:\
Ensure that each HTTP request reaching autotube's network socket is indeed an authorized one, e.g., by placing autotube behind a personal VPN or only exposing it to your LAN.


## Requirements

As autotube hands off YouTube video URLs to [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) for downloading and remuxing, **ensure that `yt-dlp` and needed dependencies (e.g., `ffmpeg`) are found in your PATH**.
Please refer to your package manager to install the required packages.


## Compilation and Running

Assuming an up-to-date Rust environment, you can compile and run autotube via:
```bash
user@machine $   git clone https://github.com/numbleroot/autotube.git
user@machine $   cd autotube
user@machine $   cargo build --release              # drop '--release' for a faster but unoptimized development build
user@machine $   ./target/release/autotube --help   # or './target/debug/autotube --help' if 'cargo build'
Download YouTube videos, automatically by following channels and on-demand by submitting URLs.

Usage: autotube [OPTIONS] --video-dir <VIDEO_DIR> --tmp-dir <TMP_DIR>

Options:
      --listen-ip <LISTEN_IP>
          The IP address the HTTP listener will bind to [env: LISTEN_IP=] [default: 127.0.0.1]
      --listen-port <LISTEN_PORT>
          The port number the HTTP listener will bind to [env: LISTEN_PORT=] [default: 22408]
      --video-dir <VIDEO_DIR>
          File system path to the location of the video directory in which videos will be placed after they have been downloaded successfully [env: VIDEO_DIR=]
      --tmp-dir <TMP_DIR>
          File system path underneath which autotube will create temporary directories for individual video download attempts [env: TMP_DIR=]
  -h, --help
          Print help
  -V, --version
          Print version
```
or directly install it via:
```bash
user@machine $   cd autotube
user@machine $   cargo install --locked --path .
user@machine $   autotube --version
autotube 0.1.0
```


## Configuration Options

autotube can be configured via the following environment and CLI arguments:
| Configuration               | ENV variable  | CLI argument    | Possible values                           | Default     |
| --------------------------- | ------------- | --------------- | ----------------------------------------- | ----------- |
| Log level                   | `RUST_LOG`    | n/a             | `TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR` | `INFO`      |
| Listen IP address           | `LISTEN_IP`   | `--listen-ip`   | any valid IP address                      | `127.0.0.1` |
| Listen port number          | `LISTEN_PORT` | `--listen-port` | any valid port number                     | `22408`     |
| Directory for videos        | `VIDEO_DIR`   | `--video-dir`   | any valid file system path                | *none*      |
| Temporary working directory | `TMP_DIR`     | `--tmp-dir`     | any valid file system path                | *none*      |


## Available HTTP Endpoints

Currently, two HTTP endpoints are serviced when autotube is running:
1. On-demand downloads: `POST /downloads/ondemand`,
2. Following YouTube channels: `POST /channels/follow`.

You can request a video to be downloaded on-demand by passing its URL in the JSON payload to `POST /downloads/ondemand`:
```bash
curl -X POST ${LISTEN_IP}:${LISTEN_PORT}/downloads/ondemand \
    --header "Content-Type: application/json" \
    --data '{ "url": "https://www.youtube.com/watch?v=<YOUTUBE_VIDEO_ID>" }'
```

After you submit a YouTube channel for following, autotube will periodically check the channel's RSS feed for any video published after you started following it.
You can specify how frequently autotube will perform these checks:
1. `"frequency": "often"` => currently set to: every 2 hours,
2. `"frequency": "sometimes"` => currently set to: every 9 hours,
3. `"frequency": "rarely"` => currently set to: every 24 hours.
Finally, you can decide how many of the most recent videos published by the YouTube channel you want to download immediately, i.e., at the time of starting to follow the channel: `"download_as_of": x`, where `0 <= x <= 255`. Note that at most the number of videos found in the channel's RSS feed can be downloaded, even if `download_as_of` was set to a higher number. Pass `"download_as_of": 0` to start downloading the YouTube channel's videos as of the next one to be published.

You can start following a YouTube channel by supplying the mentioned key-value pairs as the JSON payload in a request to `POST /downloads/ondemand`:
```bash
curl -X POST ${LISTEN_IP}:${LISTEN_PORT}/channels/follow \
    --header "Content-Type: application/json" \
    --data '{ "url": "https://www.youtube.com/@<YOUTUBE_CHANNEL>", "frequency": "sometimes", "download_as_of": 3 }'
```


## License

autotube is licensed under the [Apache-2.0 license](./LICENSE).
