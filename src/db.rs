const DATABASE_URL: &str = "file:autotube.db";

// Open connections to the SQLite database at the prescribed path. Create the
// single table `channels`, if it doesn't exist yet.
pub(crate) async fn init_db() -> anyhow::Result<sqlx::sqlite::SqlitePool> {
    let db_opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(DATABASE_URL)
        .create_if_missing(true);

    let db_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(db_opts)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS channels (
            name TEXT NOT NULL UNIQUE,
            platform TEXT NOT NULL,
            feed_url TEXT NOT NULL UNIQUE,
            check_frequency TEXT NOT NULL,
            last_checked TEXT
        ) STRICT;",
    )
    .execute(&db_pool)
    .await?;

    Ok(db_pool)
}
