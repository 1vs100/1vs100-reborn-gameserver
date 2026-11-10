use onevs100_gameserver::{
    config::{self, Config, Database},
    storage::{MemoryStorage, PostgresStorage, SqliteStorage, Storage},
};
use std::{process, sync::Arc};

fn main() {
    init_tracing(config::logging());
    let config = Config::from_env().unwrap_or_else(|errors| {
        for error in errors {
            tracing::error!("{error}");
        }
        process::exit(2);
    });
    let storage = open_storage(&config.database).unwrap_or_else(|error| {
        tracing::error!("{error}");
        process::exit(2);
    });
    if let Err(error) = onevs100_gameserver::server::run(config, storage) {
        tracing::error!("onevs100-gameserver startup failed: {error}");
        process::exit(1);
    }
}

fn init_tracing(logging: config::Logging) {
    use tracing_subscriber::EnvFilter;
    let filter = logging
        .filter
        .and_then(|filter| EnvFilter::try_new(filter).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(onevs100_gameserver::logs::open_session(&logging.dir))
        .init();
}

fn open_storage(database: &Database) -> Result<Arc<dyn Storage>, String> {
    match database {
        Database::Memory => {
            tracing::warn!(
                "GAMESERVER_DATA_DIR=memory: player data and global stats are not persisted"
            );
            Ok(Arc::new(MemoryStorage::default()))
        }
        Database::Sqlite { dir } => {
            let path = dir.join("onevs100.db");
            let storage = SqliteStorage::open(&path)
                .map_err(|error| format!("cannot open the database {}: {error}", path.display()))?;
            tracing::info!(database = %path.display(), "persistent storage ready");
            Ok(Arc::new(storage))
        }
        Database::Postgres { url } => {
            let storage = PostgresStorage::connect(url)
                .map_err(|error| format!("cannot open the PostgreSQL database: {error}"))?;
            tracing::info!(database = "postgresql", "persistent storage ready");
            Ok(Arc::new(storage))
        }
    }
}
