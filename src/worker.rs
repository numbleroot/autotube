use crate::jobs::{Job, JobCheckChannel, JobDownloadVideo, JobFollowChannel, MAX_RETRIES};
use crate::rss::{channel_get_n_most_recent_videos, channel_get_videos_as_of};
use std::os::unix::fs::DirBuilderExt;
use tracing::{Level, event};

#[allow(clippy::too_many_lines)]
// Downloads the single video pointed at in `job` by calling out to 'yt-dlp'.
// First downloads to a temporary directory under a known file name before
// moving the video to the target directory under its final name upon success.
fn download_video(state: &WorkerState, job: &JobDownloadVideo) {
    event!(Level::DEBUG, "Entering download job for {}...", job.url());

    // The temporary folder holding the downloaded video will be the current UNIX
    // epoch timestamp in microseconds, which should avoid any naming collisions due
    // to its high resolution.
    let now_unix_ms_str = chrono::Utc::now().timestamp_micros().to_string();
    let tmp_work_path = std::path::PathBuf::from(&state.tmp_dir).join(&now_unix_ms_str);
    event!(Level::DEBUG, "Creating temporary folder {tmp_work_path:?}");

    // Create temporary directory to hold video download. Will be deleted later.
    if std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&tmp_work_path)
        .is_err()
    {
        event!(
            Level::WARN,
            "Failed to create {tmp_work_path:?}, aborting job",
        );
        return;
    }

    event!(
        Level::INFO,
        "Starting download attempt {} of at most {MAX_RETRIES} for {}",
        job.attempt(),
        job.url(),
    );

    // Call out to 'yt-dlp' binary (needs to be installed) for video download.
    let Ok(ytdlp_proc) = std::process::Command::new("yt-dlp")
        .env_clear()
        .current_dir(&tmp_work_path)
        .arg("--quiet")
        .arg("--no-simulate")
        .arg("--no-warnings")
        .arg("--no-progress")
        .arg("--print")
        .arg("\"___@%(timestamp)s@___\"")
        .arg("--embed-subs")
        .arg("--embed-thumbnail")
        .arg("--embed-metadata")
        .arg("--output")
        .arg(tmp_work_path.join("download"))
        .arg(job.url())
        .output()
    else {
        event!(
            Level::WARN,
            "Process 'yt-dlp' errored with argument '{}', aborting job",
            job.url()
        );
        let _ = std::fs::remove_dir_all(&tmp_work_path);
        return;
    };

    let Ok(files_in_tmp_dir) = std::fs::read_dir(&tmp_work_path) else {
        event!(
            Level::WARN,
            "Failed to list files in {tmp_work_path:?}, aborting job"
        );
        let _ = std::fs::remove_dir_all(&tmp_work_path);
        return;
    };

    // TODO: Eventually and only if I care about non-slash file systems (Windows?),
    // I might not want to hardcode the level separator in the contains() check
    // below. Realistically, however, I won't care about them here.
    let Some(download_file_path) = &files_in_tmp_dir
        .filter_map(std::result::Result::ok)
        .filter_map(|p| p.path().into_os_string().into_string().ok())
        .find(|p| p.contains(&format!("{now_unix_ms_str}/download.")))
    else {
        // Download attempt apparently failed, as we didn't find the file we expected in
        // the created temporary working directory. As long as this job hasn't been
        // attempted too many times, resubmit it to the download queue, else discard it.

        let retry_job = match job.constr_retry() {
            Ok(j) => j,
            Err(e) => {
                event!(Level::WARN, "{e}");
                let _ = std::fs::remove_dir_all(&tmp_work_path);
                return;
            }
        };

        if (state.submit_job.blocking_send(Job::Download(retry_job))).is_err() {
            event!(
                Level::WARN,
                "Submit channel to worker queue errored, aborting job"
            );
        }
        let _ = std::fs::remove_dir_all(&tmp_work_path);
        return;
    };

    event!(
        Level::DEBUG,
        "Successful download of {}, moving to final location",
        job.url(),
    );

    let Ok(ytdlp_out) = str::from_utf8(&ytdlp_proc.stdout) else {
        event!(
            Level::WARN,
            "STDOUT from 'yt-dlp' wasn't valid UTF-8, aborting job"
        );
        let _ = std::fs::remove_dir_all(&tmp_work_path);
        return;
    };

    // Extract the video's upload timestamp from the output of the 'yt-dlp' command,
    // for use in the final name of the video file.
    let Some(video_upload_timestamp) = ytdlp_out.trim_matches(|c| c != '_').split('@').nth(1)
    else {
        event!(
            Level::WARN,
            "No upload timestamp in 'yt-dlp' output, aborting job"
        );
        let _ = std::fs::remove_dir_all(&tmp_work_path);
        return;
    };

    // Parse publication UNIX timestamp from 'yt-dlp' output to chrono DateTime.
    let Ok(published_ts) = chrono::DateTime::parse_from_str(video_upload_timestamp, "%s") else {
        event!(
            Level::WARN,
            "Unable to parse UNIX timestamp in 'yt-dlp' output, aborting job"
        );
        let _ = std::fs::remove_dir_all(&tmp_work_path);
        return;
    };

    // Convert publication UNIX timestamp to YYYY-mm-dd-HH-MM-SS format.
    let published_ts_str = published_ts.format("%Y-%m-%d-%H-%M-%S").to_string();

    // Extract the video file extension chosen by 'yt-dlp'.
    let Some((_, file_extension)) = download_file_path.rsplit_once('.') else {
        event!(
            Level::WARN,
            "No '.' in path to downloaded video, aborting job"
        );
        let _ = std::fs::remove_dir_all(&tmp_work_path);
        return;
    };

    // Construct path to final location of downloaded video file. The final name
    // consists of two parts: publication timestamp and download timestamp, allowing
    // for useful default sorting in the file system as well as avoiding name
    // collisions with overwhelming probability.
    let final_video_path = std::path::PathBuf::from(&state.video_dir).join(format!(
        "{published_ts_str}_{now_unix_ms_str}.{file_extension}"
    ));

    // Move downloaded video to final location in output directory.
    if std::fs::rename(download_file_path, final_video_path).is_err() {
        event!(
            Level::WARN,
            "Failed to move downloaded video to final location, aborting job"
        );
        let _ = std::fs::remove_dir_all(&tmp_work_path);
        return;
    }

    // Remove temporary directory created for this download attempt, including any
    // potentially leftover contained files.
    let _ = std::fs::remove_dir_all(&tmp_work_path);
    event!(Level::DEBUG, "Recursively deleted {tmp_work_path:?}");

    event!(
        Level::INFO,
        "Successfully completed video download job for {}",
        job.url(),
    );
}

// Initial steps taken for a new channel added for following to the database. If
// the download of a specific number of the channel's most recent videos is
// included in the user's request, this function kicks them off by submitting
// them as independent tasks to the queue. The `last_checked` field for the new
// channel in the database is set to the current timestamp to indicate that it
// has been handled.
fn follow_channel(state: &WorkerState, job: &JobFollowChannel) {
    event!(
        Level::DEBUG,
        "Entering follow channel job for {}...",
        job.rss_url(),
    );

    // Obtain the current timestamp in ISO 8601 / RFC 3339 format as a string.
    let now_str = chrono::Utc::now().fixed_offset().format("%+").to_string();

    // By consulting the YouTube channel's RSS feed, obtain the (potentially empty)
    // list of URLs for the `job.download_as_of` most recent published videos.
    let videos = match channel_get_n_most_recent_videos(
        &state.videos_re.clone(),
        job.rss_url(),
        job.download_as_of(),
    ) {
        Ok(v) => v,
        Err(e) => {
            event!(
                Level::WARN,
                "Worker failed to obtain recent videos for follow channel job: {e}",
            );
            return;
        }
    };

    // Insert one download job for each of the identified most recent videos.
    for video_url in videos {
        if (state
            .submit_job
            .blocking_send(Job::Download(JobDownloadVideo::new(video_url))))
        .is_err()
        {
            event!(
                Level::WARN,
                "Submit channel to worker queue errored, aborting job",
            );
            return;
        }
    }

    // Update database field indicating when we last checked for new videos by this
    // YouTube channel to the now timestamp.
    match tokio::runtime::Handle::current().block_on(async {
        let job_rss_url = job.rss_url();
        sqlx::query!(
            "UPDATE channels
            SET last_checked = $1
            WHERE feed_url = $2;",
            now_str,
            job_rss_url,
        )
        .execute(&state.db_pool)
        .await
    }) {
        Ok(_) => {
            event!(
                Level::DEBUG,
                "Worker updated 'last_checked' database field for {}",
                job.rss_url(),
            );
        }
        Err(e) => {
            event!(
                Level::WARN,
                "Worker failed to update 'last_checked' for follow channel job: {e}",
            );
            return;
        }
    }

    event!(
        Level::INFO,
        "Successfully completed follow channel job for {}, kicked of initial {} downloads",
        job.rss_url(),
        job.download_as_of(),
    );
}

#[allow(clippy::too_many_lines)]
// Triggered by a `JobCheckChannel` message on the worker queue. Checks the
// channel's RSS feed for any videos published after the `last_checked`
// timestamp found in the channel's database entry. If any newer videos are
// found, one download job each is submitted to the worker queue. Finally, the
// `last_checked` database field is set to the current timestamp (established
// upon entry to the function).
fn check_channel(state: &WorkerState, job: &JobCheckChannel) {
    event!(
        Level::DEBUG,
        "Entering check channel job for {}...",
        job.rss_url(),
    );

    // Obtain the current timestamp in ISO 8601 / RFC 3339 format as a string.
    let now_str = chrono::Utc::now().fixed_offset().format("%+").to_string();

    // Retrieve `last_checked` timestamp for this channel from database.
    let last_checked_str = match tokio::runtime::Handle::current().block_on(async {
        let job_rss_url = job.rss_url();
        sqlx::query!(
            "SELECT last_checked
            FROM channels
            WHERE feed_url = $1;",
            job_rss_url,
        )
        .fetch_one(&state.db_pool)
        .await
    }) {
        Ok(r) => {
            if let Some(l) = r.last_checked {
                l
            } else {
                event!(
                    Level::WARN,
                    "No 'last_checked' entry found for {} during check channel job, aborting job",
                    &job.rss_url(),
                );
                return;
            }
        }
        Err(e) => {
            event!(
                Level::WARN,
                "Worker failed to retrieve 'last_checked' for check channel job: {e}",
            );
            return;
        }
    };

    // Parse retrieved `last_checked` string into an RFC 3339 chrono DateTime.
    let last_checked = match chrono::DateTime::parse_from_rfc3339(&last_checked_str) {
        Ok(l) => l,
        Err(e) => {
            event!(
                Level::WARN,
                "Failed to parse 'last_checked' string to chrono DateTime, aborting job: {}",
                e,
            );
            return;
        }
    };

    // Get a (potentially empty) list of URLs for videos published at or after
    // `last_checked` from the YouTube channel's RSS feed.
    let videos = match channel_get_videos_as_of(
        &state.videos_re.clone(),
        job.rss_url(),
        last_checked,
    ) {
        Ok(v) => v,
        Err(e) => {
            event!(
                Level::WARN,
                "Worker failed to obtain videos as of {last_checked} for check channel job: {e}",
            );
            return;
        }
    };

    // Insert one download job for each of the identified new videos.
    for video_url in videos {
        if (state
            .submit_job
            .blocking_send(Job::Download(JobDownloadVideo::new(video_url))))
        .is_err()
        {
            event!(
                Level::WARN,
                "Submit channel to worker queue errored, aborting job",
            );
            return;
        }
    }

    // Update database field indicating when we last checked for new videos by this
    // YouTube channel to the now timestamp.
    match tokio::runtime::Handle::current().block_on(async {
        let job_rss_url = job.rss_url();
        sqlx::query!(
            "UPDATE channels
            SET last_checked = $1
            WHERE feed_url = $2;",
            now_str,
            job_rss_url,
        )
        .execute(&state.db_pool)
        .await
    }) {
        Ok(_) => {
            event!(
                Level::DEBUG,
                "Worker updated 'last_checked' database field for {}",
                job.rss_url(),
            );
        }
        Err(e) => {
            event!(
                Level::WARN,
                "Worker failed to update 'last_checked' for check channel job: {e}",
            );
            return;
        }
    }

    event!(
        Level::INFO,
        "Successfully completed check channel job for {}",
        job.rss_url(),
    );
}

#[derive(Clone, Debug)]
/// `WorkerState` aggregates all data that needs to be cloned into each
/// spawned blocking tasks executing one particular job from the queue.
pub(crate) struct WorkerState {
    submit_job: tokio::sync::mpsc::Sender<Job>,
    db_pool: sqlx::sqlite::SqlitePool,
    videos_re: regex::Regex,
    video_dir: String,
    tmp_dir: String,
}

impl WorkerState {
    pub(crate) fn new(
        submit_job: &tokio::sync::mpsc::Sender<Job>,
        db_pool: &sqlx::sqlite::SqlitePool,
        video_dir: String,
        tmp_dir: String,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            submit_job: submit_job.clone(),
            db_pool: db_pool.clone(),
            videos_re: regex::Regex::new(
                r#"<entry>(?s:.+?)<link rel="alternate" href="(https://www\.youtube\.com/watch\?v=.{11})"/>(?s:.+?)<published>(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+\d{2}:\d{2})</published>(?s:.+?)</entry>"#,
            )?,
            video_dir,
            tmp_dir,
        })
    }

    pub(crate) async fn run(
        self,
        mut recv_job: tokio::sync::mpsc::Receiver<Job>,
        mut recv_shutdown: tokio::sync::broadcast::Receiver<()>,
    ) {
        tokio::select! {
            _ = async {
                loop {
                    let state = self.clone();
                    if let Some(job_msg) = recv_job.recv().await {
                        match job_msg {
                            Job::Download(job) => tokio::task::spawn_blocking(move || download_video(&state, &job)),
                            Job::Follow(job) => tokio::task::spawn_blocking(move || follow_channel(&state, &job)),
                            Job::Check(job) => tokio::task::spawn_blocking(move || check_channel(&state, &job)),
                        };
                    }
                }
            } => {}
            _ = recv_shutdown.recv() => {
                event!(Level::DEBUG, "Worker shutting down...");
            }
        }
    }
}
