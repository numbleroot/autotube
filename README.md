# autotube

autotube automatically downloads new videos published by YouTube channels you chose to follow, by periodically checking the channels' RSS feeds for updates and downloading videos published since the last check by handing them off to [`yt-dlp`](https://github.com/yt-dlp/yt-dlp).

:warning: **autotube does not handle authentication.**
Ensure that any HTTP request reaching autotube's network socket is indeed an authorized one, e.g., by ensuring that the socket can only be reached from a trusted network (e.g., a LAN or VPN).


## Usage

The easiest way to deploy autotube is via the provided [`Dockerfile`](./Dockerfile).


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