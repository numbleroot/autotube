use crate::jobs::{Job, JobCheckChannel};
use rand::distr::Distribution;
use rand::prelude::SliceRandom;
use tracing::{Level, event};

enum Frequencies {
    Often,
    Sometimes,
    Rarely,
}

impl Frequencies {
    const VARIANTS: [Frequencies; 3] = [
        Frequencies::Often,
        Frequencies::Sometimes,
        Frequencies::Rarely,
    ];

    fn get_dur_mins(&self) -> u64 {
        match self {
            Frequencies::Often => 120,     // every  2 hours
            Frequencies::Sometimes => 540, // every  9 hours
            Frequencies::Rarely => 1440,   // every 24 hours
        }
    }
}

impl std::fmt::Display for Frequencies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            Frequencies::Often => write!(f, "often"),
            Frequencies::Sometimes => write!(f, "sometimes"),
            Frequencies::Rarely => write!(f, "rarely"),
        }
    }
}

// Type that represents the results returned from the below database. This is
// needed so that we can specify the input type for function
// `shuf_channels_gen_sleeps`.
struct Channel {
    feed_url: String,
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
// Perform operations involving a random number generator on the channels vector
// retrieved from the database as well as for generating the vector of durations
// (in seconds) for which the calling task will sleep for in between sending out
// check channel messages. This function scopes the `rng` variable, as the
// `ThreadRng` is not `Send` and can thus not be used across `.await` points.
fn shuf_channels_gen_sleeps(channels: &mut [Channel], dur_secs: f64) -> anyhow::Result<Vec<u64>> {
    let mut rng = rand::rng();

    // Shuffle vector of channels returned from the database so that we visit them
    // in different orders each time we check on them.
    channels.shuffle(&mut rng);

    // We'll spread the check channel message emissions across the first half of the
    // interval. In order to increase how "random" autotube's RSS feed requests
    // look, however, we'll add some jitter from (-jitter_end, jitter_end) to each
    // moment in time. Example: 3600 seconds interval with 10 channels to check on
    // in it => step_secs = 180. Thus, on average, we'll emit a message each 180
    // seconds, however, shifted by a number of seconds sampled uniformly at random
    // from (-90.0, 90.0).
    let step_secs = dur_secs / (2.0 * channels.len() as f64);
    let jitter_end = step_secs / 2.0;
    let Ok(range) = rand::distr::Uniform::new_inclusive(-jitter_end, jitter_end) else {
        return Err(anyhow::anyhow!(
            "Failed to construct random distribution over ({}, {})",
            -jitter_end,
            jitter_end,
        ));
    };

    // Compute the vector of sleep durations.
    let sleeps: Vec<u64> = range
        .sample_iter(&mut rng)
        .take(channels.len())
        .inspect(|j| println!("j={j}"))
        .map(|j| (step_secs + j).floor() as u64)
        .collect();

    Ok(sleeps)
}

#[derive(Clone, Debug)]
/// Wraps state that the time-based job trigger task needs to have access to.
pub(crate) struct TriggerState {
    submit_job: tokio::sync::mpsc::Sender<Job>,
    db_pool: sqlx::sqlite::SqlitePool,
}

impl TriggerState {
    pub(crate) fn new(
        submit_job: &tokio::sync::mpsc::Sender<Job>,
        db_pool: &sqlx::sqlite::SqlitePool,
    ) -> Self {
        TriggerState {
            submit_job: submit_job.clone(),
            db_pool: db_pool.clone(),
        }
    }

    // Once per `freq` place a check channel message per channel followed with that
    // frequency on the worker queue so that a worker task goes out and checks the
    // channel's RSS feed for any new video to download.
    async fn trigger_checks(self, freq: &Frequencies) {
        event!(Level::INFO, "Setting up trigger for frequency '{freq}'");

        // Prepare the future that will wake up exactly each `get_dur_mins()` minutes,
        // regardless of how long the computations between ticks take.
        let dur = tokio::time::Duration::from_mins(freq.get_dur_mins());
        let mut interval = tokio::time::interval(dur);
        let dur_secs = dur.as_secs_f64();

        loop {
            // Wait until the next tick has occurred.
            let _ = interval.tick().await;
            event!(Level::DEBUG, "Next tick for '{freq}' trigger occurred");

            // Retrieve all RSS feed URLs of channels marked to be checked with this
            // particular frequency from the database. Note how we exclude channels which
            // haven't been checked at all thus far (where `last_checked` == NULL). It is
            // the job of the follow channel job to conduct the initial check (including
            // potential download of videos) and initialize the `last_checked` field to its
            // first actual timestamp. This way, we prevent concurrent access issues between
            // trigger and worker tasks.
            let freq_str = freq.to_string();
            let mut channels = match sqlx::query_as!(
                Channel,
                "SELECT feed_url
                FROM channels
                WHERE check_frequency = $1 AND last_checked IS NOT NULL;",
                freq_str,
            )
            .fetch_all(&self.db_pool)
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    event!(
                        Level::WARN,
                        "Trigger failed to retrieve database items for frequency '{freq}': {e}",
                    );
                    return;
                }
            };

            if channels.is_empty() {
                event!(
                    Level::DEBUG,
                    "No channels with frequency '{freq}' in database (yet), checking back next tick...",
                );
                continue;
            }

            // Obtain the generated vector of durations to sleep between check channel
            // message emissions and also shuffle the `channels` vector.
            let sleeps = match shuf_channels_gen_sleeps(&mut channels, dur_secs) {
                Ok(j) => j,
                Err(e) => {
                    event!(Level::WARN, "Trigger failed on rand operations: {e}");
                    return;
                }
            };

            // As we only want to sleep between message emissions (and not after having sent
            // the final message for this iterator of channels), we make use of the peekable
            // version of the channels iterator, to be able to look ahead.
            let mut channels_sleeps = channels.into_iter().zip(sleeps).peekable();
            while let Some((channel, sleep)) = channels_sleeps.next() {
                if self
                    .submit_job
                    .send(Job::Check(JobCheckChannel::new(channel.feed_url)))
                    .await
                    .is_err()
                {
                    event!(
                        Level::WARN,
                        "Submit channel to worker queue errored, aborting",
                    );
                    return;
                }

                // If there's still at least one channel to come for this iterator, sleep.
                if channels_sleeps.peek().is_some() {
                    tokio::time::sleep(tokio::time::Duration::from_secs(sleep)).await;
                }
            }
        }
    }

    pub(crate) async fn run(self, mut recv_shutdown: tokio::sync::broadcast::Receiver<()>) {
        let mut set = tokio::task::JoinSet::new();
        for freq in &Frequencies::VARIANTS {
            set.spawn(self.clone().trigger_checks(freq));
        }
        let _ = recv_shutdown.recv().await;
        event!(Level::DEBUG, "Trigger shutting down...");
        let () = set.shutdown().await;
    }
}
