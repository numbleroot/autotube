use crate::jobs::{Job, JobDownloadVideo, JobFollowChannel};
use tracing::{Level, event};

#[derive(Debug, serde::Deserialize)]
pub(crate) struct DownloadsOnDemandReq {
    url: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct DownloadsOnDemandResp {
    status: String,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ChannelFollowReq {
    url: String,
    download_as_of: u8,
    frequency: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ChannelFollowResp {
    status: String,
}

#[derive(Debug, Clone)]
enum YouTubeURL {
    Video,
    Channel,
}

impl std::fmt::Display for YouTubeURL {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            YouTubeURL::Video => write!(f, "video"),
            YouTubeURL::Channel => write!(f, "channel"),
        }
    }
}

#[derive(Clone, Debug)]
/// Wraps state that each HTTP handler might need to have access to.
pub(crate) struct HTTPHandlerState {
    submit_job: tokio::sync::mpsc::Sender<Job>,
    db_pool: sqlx::sqlite::SqlitePool,
}

impl HTTPHandlerState {
    pub(crate) fn new(
        submit_job: &tokio::sync::mpsc::Sender<Job>,
        db_pool: &sqlx::sqlite::SqlitePool,
    ) -> Self {
        HTTPHandlerState {
            submit_job: submit_job.clone(),
            db_pool: db_pool.clone(),
        }
    }
}

// Verifies that everthing after 'youtube.com/watch?' in a `YouTube` video URL
// is as required, meaning that we need to find the video ID in the query
// parameters. Only used as part of validate_youtube_url, which means that we
// don't check for 'youtube.com/watch?' at the front of the URL string again.
// Returns the final, validated, full `YouTube` URL to the video.
fn validate_youtube_video_url(url: &str) -> anyhow::Result<String> {
    let url_parts = &url[18..].split('&').collect::<Vec<&str>>();

    let Some(video_id) = url_parts
        .iter()
        .find(|&&p| p.len() == 13 && p.starts_with("v="))
    else {
        event!(
            Level::DEBUG,
            "Video ID parameter missing from or incorrect in YouTube URL: {url}"
        );
        return Err(anyhow::anyhow!(
            "Video ID parameter missing from or incorrect in YouTube URL"
        ));
    };

    Ok(format!("https://www.youtube.com/watch?{video_id}"))
}

// Verifies that the submitted `YouTube` channel URL indeed links to an existing
// channel by first cleaning the URL and then making an HTTP GET request to see
// if we get a 200 OK response. If successful, extracts the RSS feed URL
// embedded on the YouTube channel webpage. Returns the final, validated, full
// `YouTube` URL to the channel and the extracted RSS feed URL.
async fn validate_youtube_channel_url(url: &str) -> anyhow::Result<(String, String)> {
    let (base_part, channel_part) = url.split_at(13);
    let channel_name = match channel_part.split_once('/') {
        Some((name, _)) => name,
        None => channel_part,
    };

    let channel_url = format!("https://www.{base_part}{channel_name}").to_lowercase();

    let Ok(resp) = reqwest::get(&channel_url).await else {
        event!(
            Level::DEBUG,
            "Failed to connect to supplied YouTube channel URL via HTTP: {channel_url}"
        );
        return Err(anyhow::anyhow!(
            "Failed to connect to supplied YouTube channel URL via HTTP"
        ));
    };

    if resp.status() != reqwest::StatusCode::OK {
        event!(
            Level::DEBUG,
            "Supplied YouTube channel URL did not return 200 OK: {channel_url}"
        );
        return Err(anyhow::anyhow!(
            "Supplied YouTube channel URL did not return 200 OK"
        ));
    }

    let Ok(channel_webpage) = resp.text().await else {
        event!(
            Level::DEBUG,
            "Unable to obtain webpage content for supplied YouTube channel URL: {channel_url}"
        );
        return Err(anyhow::anyhow!(
            "Unable to obtain webpage content for supplied YouTube channel URL"
        ));
    };

    // Find the byte position within the webpage text that signifies the start of
    // the canonical link element which contains the YouTube ID URL of the channel.
    // Manual tests have shown that this item is present in the DOM of any YouTube
    // channel webpage.
    let Some(rss_url_offset) =
        channel_webpage.find("<link rel=\"alternate\" type=\"application/rss+xml\" title=\"RSS\" href=\"https://www.youtube.com/feeds/videos.xml?channel_id=UC")
    else {
        event!(
            Level::DEBUG,
            "Didn't find channel ID in YouTube channel webpage: {channel_url}"
        );
        return Err(anyhow::anyhow!(
            "Didn't find channel ID in YouTube channel webpage"
        ));
    };

    // Extract channel ID from webpage string by extracting the right 24 characters
    // from within the webpage text.
    let rss_url = channel_webpage[(rss_url_offset + 67)..(rss_url_offset + 143)].to_string();

    Ok((channel_url, rss_url))
}

// Verifies that the supplied URL is a valid YouTube URL (either pointing to a
// video or a channel) and rejects all others. If successful, returns the
// cleaned and canonicalized version of the input URL.
async fn validate_youtube_url(kind: YouTubeURL, url: &str) -> anyhow::Result<(String, String)> {
    if url.is_empty() {
        return Err(anyhow::anyhow!(format!("Empty YouTube {kind} URL")));
    }

    let url = url.trim_start_matches("https://");
    let url = url.trim_start_matches("http://");
    let url = url.trim_start_matches("www.");

    match kind {
        YouTubeURL::Video => {
            if url.starts_with("youtube.com/watch?") {
                let valid_url = validate_youtube_video_url(url)?;
                Ok((valid_url, String::new()))
            } else {
                event!(Level::DEBUG, "Unsupported or invalid video URL: {url}");
                Err(anyhow::anyhow!("Unsupported or invalid video URL"))
            }
        }
        YouTubeURL::Channel => {
            if url.starts_with("youtube.com/@") {
                let (valid_url, channel_id) = validate_youtube_channel_url(url).await?;
                Ok((valid_url, channel_id))
            } else {
                event!(Level::DEBUG, "Unsupported or invalid channel URL: {url}");
                Err(anyhow::anyhow!("Unsupported or invalid channel URL"))
            }
        }
    }
}

/// Handle a POST request with a JSON payload containing a video URL to download
/// in the background. Currently, the only supported video platform to download
/// from is `YouTube`, any other domain is rejected as part of input validation.
pub(crate) async fn post_downloads_ondemand(
    axum::extract::State(state): axum::extract::State<HTTPHandlerState>,
    axum::Json(payload): axum::Json<DownloadsOnDemandReq>,
) -> (axum::http::StatusCode, axum::Json<DownloadsOnDemandResp>) {
    let (validated_url, _) = match validate_youtube_url(YouTubeURL::Video, &payload.url).await {
        Ok(u) => u,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(DownloadsOnDemandResp {
                    status: e.to_string(),
                }),
            );
        }
    };
    event!(
        Level::DEBUG,
        "Received valid video URL to download: {validated_url}"
    );

    // Submit validated URL via channel to a queue from which workers take URLs to
    // go and download them as videos.
    if (state
        .submit_job
        .send(Job::Download(JobDownloadVideo::new(validated_url.clone())))
        .await)
        .is_err()
    {
        event!(
            Level::DEBUG,
            "Video could not be submitted to download queue: {validated_url}"
        );
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(DownloadsOnDemandResp {
                status: "Video could not be submitted to download queue".to_string(),
            }),
        );
    }
    event!(
        Level::DEBUG,
        "Sent video URL to background process for downloading"
    );

    (
        axum::http::StatusCode::CREATED,
        axum::Json(DownloadsOnDemandResp {
            status: "Video submitted to download queue".to_string(),
        }),
    )
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn post_channels_follow(
    axum::extract::State(state): axum::extract::State<HTTPHandlerState>,
    axum::Json(payload): axum::Json<ChannelFollowReq>,
) -> (axum::http::StatusCode, axum::Json<ChannelFollowResp>) {
    let frequency = match payload.frequency.as_str() {
        "often" => "often",
        "sometimes" => "sometimes",
        "rarely" => "rarely",
        &_ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(ChannelFollowResp {
                    status: "Field 'argument' needs to be one of: 'often', 'sometimes', 'rarely'"
                        .to_string(),
                }),
            );
        }
    };

    let (validated_url, channel_rss) =
        match validate_youtube_url(YouTubeURL::Channel, &payload.url).await {
            Ok(u) => u,
            Err(e) => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(ChannelFollowResp {
                        status: e.to_string(),
                    }),
                );
            }
        };
    event!(
        Level::DEBUG,
        "Received valid channel URL to follow: {validated_url}"
    );

    // Enter YouTube channel with metadata into table tracking channels.
    match sqlx::query!(
        "INSERT INTO channels ( name, platform, feed_url, check_frequency )
        VALUES ( $1, $2, $3, $4 );",
        validated_url,
        "youtube",
        channel_rss,
        frequency,
    )
    .execute(&state.db_pool)
    .await
    {
        Ok(_) => {}
        Err(e) => match e {
            sqlx::Error::Database(err_db) if err_db.is_unique_violation() => {
                event!(
                    Level::DEBUG,
                    "Submitted channel is already being followed: {validated_url}"
                );
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(ChannelFollowResp {
                        status: "Submitted channel is already being followed".to_string(),
                    }),
                );
            }
            _ => {
                event!(
                    Level::WARN,
                    "Inserting new channel to follow into database failed: {e}"
                );
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(ChannelFollowResp {
                        status: "Inserting new channel to follow into database failed".to_string(),
                    }),
                );
            }
        },
    }

    if (state
        .submit_job
        .send(Job::Follow(JobFollowChannel::new(
            channel_rss.clone(),
            payload.download_as_of,
        )))
        .await)
        .is_err()
    {
        event!(
            Level::DEBUG,
            "Initial download of new channel could not be sent to queue: {validated_url}"
        );
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(ChannelFollowResp {
                status: "Initial download of new channel could not be sent to queue".to_string(),
            }),
        );
    }
    event!(
        Level::DEBUG,
        "Sent channel following job to background process for initial downloads (if requested)"
    );

    (
        axum::http::StatusCode::CREATED,
        axum::Json(ChannelFollowResp {
            status: format!("Started following channel {validated_url}"),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_validate_video_urls() {
        // Below URL inputs to validate_youtube_url() should all produce an Error
        // result with the associated error message.
        let should_error = [
            ("", "Unsupported or invalid video URL"),
            ("abc", "Unsupported or invalid video URL"),
            ("http://vimeo.com", "Unsupported or invalid video URL"),
            ("https://www.google.com", "Unsupported or invalid video URL"),
            (
                "youtube.org/watch?v=0123456789a",
                "Unsupported or invalid video URL",
            ),
            (
                "https://www.youtube.com/watch?v=0123456789",
                "Video ID parameter missing from or incorrect in YouTube URL",
            ),
            (
                "https://www.youtube.com/watch?v=0123456789ab",
                "Video ID parameter missing from or incorrect in YouTube URL",
            ),
            (
                "https://www.youtube.com/watch?k=0123456789a",
                "Video ID parameter missing from or incorrect in YouTube URL",
            ),
            (
                "https://www.youtube.com/watch?v=0123456789&list=abcdefghijklmnopqrstuvwxyzeRgBdnBM",
                "Video ID parameter missing from or incorrect in YouTube URL",
            ),
        ];

        for (url, exp_err) in &should_error {
            assert!(
                validate_youtube_url(YouTubeURL::Video, url)
                    .await
                    .is_err_and(|e| e.to_string() == *exp_err)
            );
        }

        // Below URL inputs to validate_youtube_url() should all produce an Ok result
        // with the associated valid URL returned.
        let should_succeed = [
            (
                "youtube.com/watch?v=0123456789a",
                "https://www.youtube.com/watch?v=0123456789a",
            ),
            (
                "www.youtube.com/watch?v=0123456789a",
                "https://www.youtube.com/watch?v=0123456789a",
            ),
            (
                "http://youtube.com/watch?v=0123456789a",
                "https://www.youtube.com/watch?v=0123456789a",
            ),
            (
                "http://www.youtube.com/watch?v=0123456789a",
                "https://www.youtube.com/watch?v=0123456789a",
            ),
            (
                "https://www.youtube.com/watch?v=0123456789a",
                "https://www.youtube.com/watch?v=0123456789a",
            ),
            (
                "https://www.youtube.com/watch?v=0123456789a&",
                "https://www.youtube.com/watch?v=0123456789a",
            ),
            (
                "https://www.youtube.com/watch?v=0123456789a&other=ignored&more=alsoignored",
                "https://www.youtube.com/watch?v=0123456789a",
            ),
        ];

        for (url, exp_ret) in &should_succeed {
            assert!(
                validate_youtube_url(YouTubeURL::Video, url)
                    .await
                    .is_ok_and(|(u, _)| u == *exp_ret)
            );
        }
    }
}
