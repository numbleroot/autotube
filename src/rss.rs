// Return the list of videos found in the YouTube channel's RSS feed as tuples
// <publication timestamp, video URL>, sorted from most recent to least recent.
fn channel_get_most_recent_videos(
    videos_re: &regex::Regex,
    rss_url: &str,
) -> anyhow::Result<Vec<(chrono::DateTime<chrono::FixedOffset>, String)>> {
    // Obtain the the YouTube channel's RSS feed using reqwest's blocking GET
    // function and extract the body as text.
    let rss_data = reqwest::blocking::get(rss_url)?.text()?;

    // Extract the <publication date, video URL> tuple for all videos found
    // wrapped inside <entry></entry> in the YouTube channel's RSS feed.
    let mut videos: Vec<(chrono::DateTime<chrono::FixedOffset>, String)> = vec![];
    for (_, [video_url, pub_date]) in videos_re.captures_iter(&rss_data).map(|c| c.extract()) {
        let Ok(parsed_pub_date) = pub_date.parse::<chrono::DateTime<chrono::FixedOffset>>() else {
            return Err(anyhow::anyhow!(format!(
                "Couldn't parse publication date {pub_date} into valid chrono date"
            )));
        };

        videos.push((parsed_pub_date, video_url.to_string()));
    }

    // Sort tuple vector by publication date entries, newest to oldest.
    videos.sort_by(|(t1, _), (t2, _)| t2.cmp(t1));

    Ok(videos)
}

// From the sorted list of videos of a YouTube channel, return the URLs to the
// `num_items` most recent ones.
pub(crate) fn channel_get_n_most_recent_videos(
    videos_re: &regex::Regex,
    rss_url: &str,
    num_items: u8,
) -> anyhow::Result<Vec<String>> {
    // Obtain sorted list of <publication timestamp, video URL> tuples of channel.
    let most_recent_videos = channel_get_most_recent_videos(videos_re, rss_url)?;

    // Select only the specified number of items from the front of sorted videos
    // list and discard the publication times, leaving only their URLs.
    let (_, n_most_recent_videos): (Vec<chrono::DateTime<chrono::FixedOffset>>, Vec<String>) =
        most_recent_videos
            .into_iter()
            .take(num_items.into())
            .unzip();

    Ok(n_most_recent_videos)
}

// From the sorted list of videos of a YouTube channel, return the URLs to the
// ones that were published at or after the `as_of` timestamp.
pub(crate) fn channel_get_videos_as_of(
    videos_re: &regex::Regex,
    rss_url: &str,
    as_of: chrono::DateTime<chrono::FixedOffset>,
) -> anyhow::Result<Vec<String>> {
    // Obtain sorted list of <publication timestamp, video URL> tuples of channel.
    let most_recent_videos = channel_get_most_recent_videos(videos_re, rss_url)?;

    // Select only the videos from the sorted list that were published at or after
    // the supplied `as_of` timestamp and discard the publication times, leaving
    // only their URLs.
    let (_, videos_as_of): (Vec<chrono::DateTime<chrono::FixedOffset>>, Vec<String>) =
        most_recent_videos
            .into_iter()
            .filter(|(t, _)| t >= &as_of)
            .unzip();

    Ok(videos_as_of)
}
