use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, FromRow)]
pub struct Thread {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
    pub summary: String,
    #[allow(dead_code)]
    pub created_at: String,
    pub updated_at: String,
}

pub async fn create_thread(pool: &SqlitePool, user_id: i64) -> Result<Thread, sqlx::Error> {
    let id = sqlx::query(r#"INSERT INTO thread (user_id, title, summary) VALUES (?, '', '')"#)
        .bind(user_id)
        .execute(pool)
        .await?
        .last_insert_rowid();
    set_current_thread(pool, user_id, Some(id)).await?;
    get_thread(pool, id).await
}

pub async fn get_thread(pool: &SqlitePool, id: i64) -> Result<Thread, sqlx::Error> {
    sqlx::query_as::<_, Thread>(
        r#"SELECT id, user_id, title, summary, created_at, updated_at
           FROM thread WHERE id = ?"#,
    )
    .bind(id)
    .fetch_one(pool)
    .await
}

pub async fn list_threads(pool: &SqlitePool, user_id: i64) -> Result<Vec<Thread>, sqlx::Error> {
    sqlx::query_as::<_, Thread>(
        r#"SELECT id, user_id, title, summary, created_at, updated_at
           FROM thread WHERE user_id = ? ORDER BY updated_at DESC, id DESC"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn update_summary(
    pool: &SqlitePool,
    id: i64,
    summary: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE thread
           SET summary = ?,
               updated_at = STRFTIME('%Y-%m-%d %H:%M:%f', 'NOW')
           WHERE id = ?"#,
    )
    .bind(summary)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_title_if_empty(
    pool: &SqlitePool,
    id: i64,
    title: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE thread
           SET title = ?, updated_at = STRFTIME('%Y-%m-%d %H:%M:%f', 'NOW')
           WHERE id = ? AND title = ''"#,
    )
    .bind(title)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_thread(
    pool: &SqlitePool,
    user_id: i64,
    thread_id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(r#"DELETE FROM thread WHERE id = ? AND user_id = ?"#)
        .bind(thread_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_all_threads(pool: &SqlitePool, user_id: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(r#"DELETE FROM thread WHERE user_id = ?"#)
        .bind(user_id)
        .execute(pool)
        .await?;
    set_current_thread(pool, user_id, None).await?;
    Ok(result.rows_affected())
}

pub async fn get_current_thread_id(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let id: Option<Option<i64>> = sqlx::query_scalar(
        r#"SELECT current_thread_id FROM user_state WHERE user_id = ?"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(id.flatten())
}

pub async fn set_current_thread(
    pool: &SqlitePool,
    user_id: i64,
    thread_id: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO user_state (user_id, current_thread_id) VALUES (?, ?)
           ON CONFLICT(user_id) DO UPDATE SET current_thread_id = excluded.current_thread_id"#,
    )
    .bind(user_id)
    .bind(thread_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn enabled_skills(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<std::collections::HashSet<String>, sqlx::Error> {
    let names: Vec<String> = sqlx::query_scalar(
        r#"SELECT skill FROM user_skill WHERE user_id = ? AND enabled = 1"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(names.into_iter().collect())
}

pub async fn set_skill(
    pool: &SqlitePool,
    user_id: i64,
    skill: &str,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    let flag = if enabled { 1 } else { 0 };
    sqlx::query(
        r#"INSERT INTO user_skill (user_id, skill, enabled) VALUES (?, ?, ?)
           ON CONFLICT(user_id, skill) DO UPDATE SET enabled = excluded.enabled"#,
    )
    .bind(user_id)
    .bind(skill)
    .bind(flag)
    .execute(pool)
    .await?;
    Ok(())
}

impl Thread {
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            "(untitled)"
        } else {
            self.title.as_str()
        }
    }
}
