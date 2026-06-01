//! Categories with extension-based auto-categorize rules.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::error::{CoreError, Result};

pub type CategoryId = i64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
    pub icon: Option<String>,
    pub default_output_path: Option<PathBuf>,
    /// Lower-cased file extensions, no leading dot.
    pub extension_rules: Vec<String>,
}

/// Inputs to [`crate::Core::add_category`] / [`crate::Core::update_category`].
#[derive(Debug, Clone)]
pub struct NewCategory {
    pub name: String,
    pub icon: Option<String>,
    pub default_output_path: Option<PathBuf>,
    pub extension_rules: Vec<String>,
}

pub(crate) async fn list(pool: &SqlitePool) -> Result<Vec<Category>> {
    let rows = sqlx::query("SELECT * FROM categories ORDER BY display_order ASC, id ASC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(category_from_row).collect()
}

pub(crate) async fn get(pool: &SqlitePool, id: CategoryId) -> Result<Category> {
    let row = sqlx::query("SELECT * FROM categories WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or(CoreError::CategoryNotFound(id))?;
    category_from_row(&row)
}

pub(crate) async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Category>> {
    let row = sqlx::query("SELECT * FROM categories WHERE name = ?")
        .bind(name)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(category_from_row).transpose()
}

pub(crate) async fn insert(pool: &SqlitePool, input: NewCategory) -> Result<CategoryId> {
    let exts = serde_json::to_string(&normalize_extensions(&input.extension_rules))?;
    let folder = input
        .default_output_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let row = sqlx::query(
        "INSERT INTO categories (name, icon, default_output_path, extension_rules, display_order) \
         VALUES (?, ?, ?, ?, (SELECT COALESCE(MAX(display_order), 0) + 1 FROM categories)) \
         RETURNING id",
    )
    .bind(&input.name)
    .bind(&input.icon)
    .bind(folder)
    .bind(exts)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

/// Rewrite the `display_order` column to match the supplied order. The
/// supplied id set must equal the current id set, else this is rejected.
/// Runs in a single transaction so partial updates aren't observable.
pub(crate) async fn set_order(pool: &SqlitePool, ids: &[CategoryId]) -> Result<()> {
    let mut tx = pool.begin().await?;

    let current: Vec<CategoryId> = sqlx::query_scalar("SELECT id FROM categories")
        .fetch_all(&mut *tx)
        .await?;
    let mut want: Vec<CategoryId> = ids.to_vec();
    let mut have = current.clone();
    want.sort_unstable();
    have.sort_unstable();
    if want != have {
        return Err(CoreError::InvalidArgument(
            "id set does not match current categories".into(),
        ));
    }

    for (index, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE categories SET display_order = ? WHERE id = ?")
            .bind(index as i64 + 1)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub(crate) async fn update(pool: &SqlitePool, id: CategoryId, input: NewCategory) -> Result<()> {
    let exts = serde_json::to_string(&normalize_extensions(&input.extension_rules))?;
    let folder = input
        .default_output_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let res = sqlx::query(
        "UPDATE categories SET name = ?, icon = ?, default_output_path = ?, extension_rules = ? \
         WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.icon)
    .bind(folder)
    .bind(exts)
    .bind(id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::CategoryNotFound(id));
    }
    Ok(())
}

pub(crate) async fn remove(pool: &SqlitePool, id: CategoryId) -> Result<()> {
    let res = sqlx::query("DELETE FROM categories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::CategoryNotFound(id));
    }
    Ok(())
}

/// Resolve a filename to a category id by extension match. Returns `None`
/// if no rules match — the caller falls back to "Other" (which always
/// exists from the seed migration).
pub(crate) async fn auto_categorize_for_filename(
    pool: &SqlitePool,
    filename: &str,
) -> Result<Option<CategoryId>> {
    let ext = match extract_extension(filename) {
        Some(e) => e,
        None => return Ok(other_category_id(pool).await.ok()),
    };
    let cats = list(pool).await?;
    for c in &cats {
        if c.extension_rules.iter().any(|r| r == &ext) {
            return Ok(Some(c.id));
        }
    }
    Ok(other_category_id(pool).await.ok())
}

async fn other_category_id(pool: &SqlitePool) -> Result<CategoryId> {
    let row = sqlx::query("SELECT id FROM categories WHERE name = 'Other'")
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or_else(|| CoreError::CategoryNameNotFound("Other".to_string()))?;
    Ok(row.get("id"))
}

fn category_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Category> {
    let exts: String = row.get("extension_rules");
    let exts: Vec<String> = serde_json::from_str(&exts)?;
    let folder: Option<String> = row.get("default_output_path");
    Ok(Category {
        id: row.get("id"),
        name: row.get("name"),
        icon: row.get("icon"),
        default_output_path: folder.map(PathBuf::from),
        extension_rules: exts,
    })
}

fn extract_extension(filename: &str) -> Option<String> {
    let dot = filename.rfind('.')?;
    let ext = &filename[dot + 1..];
    if ext.is_empty() || ext.contains('/') || ext.contains('\\') {
        None
    } else {
        Some(ext.to_lowercase())
    }
}

fn normalize_extensions(input: &[String]) -> Vec<String> {
    input
        .iter()
        .map(|e| e.trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn set_order_rewrites_display_order() {
        let pool = fresh_pool().await;
        let initial = list(&pool).await.unwrap();
        assert!(initial.len() >= 2);

        let reversed: Vec<CategoryId> = initial.iter().rev().map(|c| c.id).collect();
        set_order(&pool, &reversed).await.unwrap();

        let after = list(&pool).await.unwrap();
        let after_ids: Vec<CategoryId> = after.iter().map(|c| c.id).collect();
        assert_eq!(after_ids, reversed);
    }

    #[tokio::test]
    async fn set_order_rejects_mismatched_id_set() {
        let pool = fresh_pool().await;
        let initial = list(&pool).await.unwrap();
        let mut ids: Vec<CategoryId> = initial.iter().map(|c| c.id).collect();
        ids.pop();
        let res = set_order(&pool, &ids).await;
        assert!(matches!(res, Err(CoreError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn insert_assigns_next_display_order() {
        let pool = fresh_pool().await;
        let before = list(&pool).await.unwrap();
        let max_before = before.len() as i64;
        let new_id = insert(
            &pool,
            NewCategory {
                name: "eBooks".into(),
                icon: Some("book".into()),
                default_output_path: None,
                extension_rules: vec!["epub".into(), "mobi".into()],
            },
        )
        .await
        .unwrap();
        let order: i64 = sqlx::query_scalar("SELECT display_order FROM categories WHERE id = ?")
            .bind(new_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(order, max_before + 1);
    }

    #[test]
    fn extension_extraction() {
        assert_eq!(extract_extension("Foo.MP3").as_deref(), Some("mp3"));
        assert_eq!(extract_extension("archive.tar.gz").as_deref(), Some("gz"));
        assert_eq!(extract_extension("noext"), None);
        assert_eq!(extract_extension(".hidden").as_deref(), Some("hidden"));
    }

    #[test]
    fn extensions_normalized() {
        let v = normalize_extensions(&[".MP3".into(), "WAV".into(), "".into(), ".".into()]);
        assert_eq!(v, vec!["mp3", "wav"]);
    }

    /// Regression for the yt-dlp finalize flow: an extensionless title
    /// falls into "Other" (matching what insert sees), but once yt-dlp
    /// has supplied the `.mp4`, the same filename routes to "Video".
    /// The queue worker uses this difference to flip the row's
    /// `category_id` on completion.
    #[tokio::test]
    async fn auto_categorize_routes_extensionless_to_other_and_mp4_to_video() {
        let pool = fresh_pool().await;
        let other = find_by_name(&pool, "Other").await.unwrap().unwrap();
        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();

        let without_ext = auto_categorize_for_filename(&pool, "SECSUN Articulated Boom Lift")
            .await
            .unwrap();
        let with_mp4 = auto_categorize_for_filename(&pool, "SECSUN Articulated Boom Lift.mp4")
            .await
            .unwrap();

        assert_eq!(without_ext, Some(other.id));
        assert_eq!(with_mp4, Some(video.id));
        assert_ne!(without_ext, with_mp4);
    }
}
