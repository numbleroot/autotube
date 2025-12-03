use crate::handlers::{HTTPHandlerState, post_channels_follow, post_downloads_ondemand};
use crate::jobs::Job;
use crate::trigger::TriggerState;
use crate::worker::WorkerState;
use clap::Parser;
use tracing::{Level, event};
use tracing_subscriber::prelude::*;

mod db;
mod handlers;
mod jobs;
mod rss;
mod trigger;
mod worker;

#[derive(Debug, Parser)]
#[command(about, author, version, next_line_help = true)]
struct Args {
    #[arg(long, env, default_value = "127.0.0.1")]
    /// The IP address the HTTP listener will bind to.
    listen_ip: String,

    #[arg(long, env, default_value = "22408")]
    /// The port number the HTTP listener will bind to.
    listen_port: String,

    #[arg(long, env)]
    /// File system path to the location of the video directory in which videos
    /// will be placed after they have been downloaded successfully.
    video_dir: String,

    #[arg(long, env)]
    /// File system path underneath which autotube will create temporary
    /// directories for individual video download attempts.
    tmp_dir: String,
}

// Wait to observe the ctrl+c signal and cause everything to shut down properly
// by dropping the sender half of a broadcast channel (all receivers will close
// upon this event).
async fn shutdown_upon_signal(send_shutdown: tokio::sync::broadcast::Sender<()>) {
    let _ = tokio::signal::ctrl_c().await;
    event!(Level::INFO, "Received signal to shut down gracefully");
    drop(send_shutdown);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI and ENV arguments.
    let args = Args::parse();

    // Configure our tracing/logger.
    let format_layer = tracing_subscriber::fmt::layer()
        .with_file(true)
        .with_line_number(true)
        .compact();
    let filter_layer = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new("info"))?;
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(format_layer)
        .init();
    event!(Level::DEBUG, "Launching...");

    // Error out early on if `yt-dlp` can't be called from autotube.
    if std::process::Command::new("yt-dlp")
        .env_clear()
        .current_dir(&args.tmp_dir)
        .arg("--version")
        .output()
        .is_err()
    {
        return Err(anyhow::anyhow!(
            "No 'yt-dlp' executable found, make sure it is installed"
        ));
    }

    // Initialize a connection to the SQLite database and also create the primary
    // table if it doesn't exist.
    let db_pool = db::init_db().await?;

    // Prepare ctrl+c signal handling: Spawn a background task waiting for ctrl+c
    // being pressend to then drop the sender side of a broadcast channel to which
    // all other tasks are hooked up as receivers. As soon as the receivers see the
    // sender getting dropped, they initiate shutdown.
    let (send_shutdown, _) = tokio::sync::broadcast::channel::<()>(1);

    // Prepare an MPSC channel pair with a decent buffer size for HTTP handlers to
    // submit jobs to a (blocking) background process to execute.
    let (submit_job, recv_job) = tokio::sync::mpsc::channel::<Job>(256);

    // The job sender end goes into the state struct that will be passed to each
    // HTTP request handler axum will spawn.
    let handler_state = HTTPHandlerState::new(&submit_job, &db_pool);

    // Run the background task triggering the check for new videos on any of the
    // followed channels and also provide it access to the job queue and the
    // database.
    let trigger_state = TriggerState::new(&submit_job, &db_pool);
    let trigger_shutdown = send_shutdown.subscribe();
    let trigger_handle = tokio::task::spawn(trigger_state.run(trigger_shutdown));

    let worker_state = WorkerState::new(&submit_job, &db_pool, args.video_dir, args.tmp_dir)?;
    let worker_shutdown = send_shutdown.subscribe();
    let worker_handle = tokio::task::spawn(worker_state.run(recv_job, worker_shutdown));

    // Build HTTP router to handle incoming client requests. Note that we assume to
    // be running behind a security perimeter (e.g., WireGuard), so that
    // authentication is not a concern for us.
    let router = axum::Router::new()
        .without_v07_checks()
        .route(
            "/downloads/ondemand",
            axum::routing::post(post_downloads_ondemand),
        )
        .route(
            "/channels/follow",
            axum::routing::post(post_channels_follow),
        )
        .with_state(handler_state);

    // Spawn a tokio TCP listener on the configured listening IP and port, and pass
    // it off to axum to handle the configured HTTP routes.
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", args.listen_ip, args.listen_port)).await?;
    event!(
        Level::INFO,
        "Listening for HTTP requests on {}:{}...",
        args.listen_ip,
        args.listen_port
    );

    // Block on HTTP handler, returning upon shutdown.
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_upon_signal(send_shutdown))
        .await?;

    // Once HTTP handler completed, also wait for background tasks and database
    // connections to exit.
    let _ = worker_handle.await;
    trigger_handle.await?;
    db_pool.close().await;

    Ok(())
}
