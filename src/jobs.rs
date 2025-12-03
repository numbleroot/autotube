pub(crate) const MAX_RETRIES: u8 = 3;

#[derive(Clone, Debug)]
/// Instruct the background worker task to download the enclosed `YouTube`
/// video. If failing to do so, autotube will try to download the video at most
/// `MAX_RETRIES` number of times.
pub(crate) struct JobDownloadVideo {
    url: String,
    attempt: u8,
}

impl JobDownloadVideo {
    pub(crate) fn new(url: String) -> JobDownloadVideo {
        Self { url, attempt: 1 }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn attempt(&self) -> u8 {
        self.attempt
    }

    pub(crate) fn constr_retry(&self) -> anyhow::Result<JobDownloadVideo> {
        if self.attempt < MAX_RETRIES {
            Ok(Self {
                url: self.url.clone(),
                attempt: self.attempt + 1,
            })
        } else {
            Err(anyhow::anyhow!(format!(
                "Unsucessfully tried {MAX_RETRIES} times to download {}, aborting job",
                &self.url
            )))
        }
    }
}

#[derive(Clone, Debug)]
/// Instruct autotube to start following the video releases of the `YouTube`
/// channel at the enclosed URL. Potentially start downloading a number of the
/// channel's most recent videos as well.
pub(crate) struct JobFollowChannel {
    rss_url: String,
    download_as_of: u8,
}

impl JobFollowChannel {
    pub(crate) fn new(rss_url: String, download_as_of: u8) -> JobFollowChannel {
        Self {
            rss_url,
            download_as_of,
        }
    }

    pub(crate) fn rss_url(&self) -> &str {
        &self.rss_url
    }

    pub(crate) fn download_as_of(&self) -> u8 {
        self.download_as_of
    }
}

#[derive(Clone, Debug)]
pub(crate) struct JobCheckChannel {
    rss_url: String,
}

impl JobCheckChannel {
    pub(crate) fn new(rss_url: String) -> JobCheckChannel {
        Self { rss_url }
    }

    pub(crate) fn rss_url(&self) -> &str {
        &self.rss_url
    }
}

#[derive(Clone, Debug)]
/// `Job` encapsulates a variant for each of the different (long-running,
/// synchronous, blocking) tasks a background worker listening for them on a
/// channel might be assigned.
pub(crate) enum Job {
    Download(JobDownloadVideo),
    Follow(JobFollowChannel),
    Check(JobCheckChannel),
}
