//! `suertudo-db` — the database command line tool.
//!
//! `scripts/dev.sh` calls `cargo run --release -p suertudo-db --bin suertudo-db -- migrate`
//! to bring a database up to date before the workers start.

use std::process::ExitCode;

use suertudo_db::pool;

const USAGE: &str = "\
suertudo-db <command>

Commands:
  migrate   Apply every pending migration from services/db/migrations
  help      Show this message

Environment:
  DATABASE_URL   Connection string of the database to migrate (required)
";

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();

    match command.as_str() {
        "migrate" => match migrate().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("suertudo-db: migration failed: {err}");
                ExitCode::FAILURE
            }
        },
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        "" => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
        other => {
            eprintln!("suertudo-db: unknown command `{other}`\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// Applies the pending migrations to `DATABASE_URL`.
async fn migrate() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL is required to run the migrations")?;

    let pool = pool::connect(&url, 5).await?;
    pool::migrate(&pool).await?;

    println!("migrations applied to {url}");
    Ok(())
}
