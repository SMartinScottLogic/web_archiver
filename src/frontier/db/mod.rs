pub mod schema;

use rusqlite::{Connection, Result};

pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;

        conn.busy_timeout(std::time::Duration::from_secs(30))?;

        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA temp_store=MEMORY;
            ",
        )?;

        schema::init_schema(&conn)?;

        Ok(Self { conn })
    }
}
