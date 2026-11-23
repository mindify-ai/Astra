//! Postgres op log.

use astra_protocol::{Op, OpBody};
use astra_sync::{OpLog, SyncError};
use sqlx::postgres::PgPool;
use sqlx::types::Json;
use sqlx::Row;

/// Op log in Postgres. Sequence numbers come from a per-workspace counter row locked with
/// `SELECT ... FOR UPDATE`, so concurrent appends (even from several server instances) stay gap-free.
#[derive(Clone)]
pub struct PgOpLog {
    pool: PgPool,
}

fn db(e: sqlx::Error) -> SyncError {
    SyncError::Storage(e.to_string())
}

impl PgOpLog {
    /// An op log over `pool` (run [`migrate`] first).
    pub fn new(pool: PgPool) -> Self {
        PgOpLog { pool }
    }
}

/// Run the embedded migrations.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

#[async_trait::async_trait]
impl OpLog for PgOpLog {
    async fn append(&self, workspace: &str, ops: &[Op]) -> astra_sync::Result<Vec<(u64, Op)>> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("INSERT INTO workspaces (id) VALUES ($1) ON CONFLICT (id) DO NOTHING")
            .bind(workspace)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let mut next: i64 = sqlx::query("SELECT next_seq FROM workspaces WHERE id = $1 FOR UPDATE")
            .bind(workspace)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?
            .try_get(0)
            .map_err(db)?;
        let mut stored = Vec::new();
        for op in ops {
            let body = serde_json::to_value(&op.body).map_err(|e| SyncError::Codec(e.to_string()))?;
            let inserted = sqlx::query(
                "INSERT INTO ops (workspace_id, seq, op_id, hlc, actor, body) VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (op_id) DO NOTHING",
            )
            .bind(workspace)
            .bind(next)
            .bind(&op.id)
            .bind(op.hlc.to_string())
            .bind(&op.actor)
            .bind(Json(body))
            .execute(&mut *tx)
            .await
            .map_err(db)?
            .rows_affected();
            if inserted == 1 {
                stored.push((u64::try_from(next).unwrap_or(0), op.clone()));
                next += 1;
            }
        }
        sqlx::query("UPDATE workspaces SET next_seq = $2 WHERE id = $1")
            .bind(workspace)
            .bind(next)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(stored)
    }

    async fn since(&self, workspace: &str, cursor: u64, limit: usize) -> astra_sync::Result<Vec<(u64, Op)>> {
        let rows = sqlx::query(
            "SELECT seq, op_id, hlc, actor, body FROM ops WHERE workspace_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
        )
        .bind(workspace)
        .bind(i64::try_from(cursor).unwrap_or(i64::MAX))
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter()
            .map(|r| {
                let seq: i64 = r.try_get(0).map_err(db)?;
                let hlc: String = r.try_get(2).map_err(db)?;
                let Json(body): Json<serde_json::Value> = r.try_get(4).map_err(db)?;
                let op = Op {
                    id: r.try_get(1).map_err(db)?,
                    hlc: hlc.parse().map_err(|e: astra_protocol::HlcParseError| SyncError::Storage(e.to_string()))?,
                    actor: r.try_get(3).map_err(db)?,
                    body: serde_json::from_value::<OpBody>(body).map_err(|e| SyncError::Codec(e.to_string()))?,
                };
                Ok((u64::try_from(seq).unwrap_or(0), op))
            })
            .collect()
    }

    async fn head(&self, workspace: &str) -> astra_sync::Result<u64> {
        let row = sqlx::query("SELECT next_seq - 1 FROM workspaces WHERE id = $1")
            .bind(workspace)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(match row {
            Some(r) => u64::try_from(r.try_get::<i64, _>(0).map_err(db)?).unwrap_or(0),
            None => 0,
        })
    }

    async fn node_seen(&self, workspace: &str, node: &str) -> astra_sync::Result<()> {
        sqlx::query("INSERT INTO workspaces (id) VALUES ($1) ON CONFLICT (id) DO NOTHING")
            .bind(workspace)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        sqlx::query(
            "INSERT INTO nodes (id, workspace_id, last_seen) VALUES ($1, $2, now())
             ON CONFLICT (workspace_id, id) DO UPDATE SET last_seen = now()",
        )
        .bind(node)
        .bind(workspace)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }
}

/// Record a person (from OAuth) and return the workspaces they belong to.
pub async fn upsert_person(
    pool: &PgPool,
    id: &str,
    name: &str,
    provider: &str,
    provider_id: &str,
    personal_workspace: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO people (id, name, provider, provider_id) VALUES ($1, $2, $3, $4)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name",
    )
    .bind(id)
    .bind(name)
    .bind(provider)
    .bind(provider_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO workspaces (id, name) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING")
        .bind(personal_workspace)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO memberships (workspace_id, person_id, role) VALUES ($1, $2, 'owner') ON CONFLICT DO NOTHING",
    )
    .bind(personal_workspace)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let rows = sqlx::query("SELECT workspace_id FROM memberships WHERE person_id = $1 ORDER BY workspace_id")
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    rows.into_iter().map(|r| r.try_get(0)).collect()
}
