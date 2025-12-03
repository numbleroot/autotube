# autotube

autotube automatically downloads new videos published by YouTube channels you chose to follow, by periodically checking the channels' RSS feeds for updates and downloading videos published since the last check by handing them off to [`yt-dlp`](https://github.com/yt-dlp/yt-dlp).

autotube does not perform authentication.
Any well-formed HTTP request reaching autotube's endpoint is dutifully handled.
Take necessary precautions, e.g., by placing autotube behind your existing authentication solution or VPN.


## Usage

The easiest way to deploy autotube is via the provided [`Dockerfile`](./Dockerfile).


## License

autotube is licensed under the [Apache-2.0 license](./LICENSE).