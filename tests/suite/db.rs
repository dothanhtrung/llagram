use crate::common;
use llagram::db;

#[tokio::test]
async fn create_list_and_current_thread() {
    let db = common::test_db().await;
    let a = db::create_thread(&db.pool, 7).await.unwrap();
    let b = db::create_thread(&db.pool, 7).await.unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(a.user_id, 7);
    assert!(a.title.is_empty());
    assert!(a.summary.is_empty());
    assert_eq!(a.display_title(), "(untitled)");

    let listed = db::list_threads(&db.pool, 7).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, b.id, "newest thread is first");

    assert_eq!(
        db::get_current_thread_id(&db.pool, 7).await.unwrap(),
        Some(b.id)
    );
    assert!(db::list_threads(&db.pool, 8).await.unwrap().is_empty());
}

#[tokio::test]
async fn title_only_fills_when_empty_and_summary_updates() {
    let db = common::test_db().await;
    let thread = db::create_thread(&db.pool, 1).await.unwrap();
    db::set_title_if_empty(&db.pool, thread.id, "hello world")
        .await
        .unwrap();
    db::set_title_if_empty(&db.pool, thread.id, "ignored")
        .await
        .unwrap();
    db::update_summary(&db.pool, thread.id, "remember this")
        .await
        .unwrap();

    let loaded = db::get_thread(&db.pool, thread.id).await.unwrap();
    assert_eq!(loaded.title, "hello world");
    assert_eq!(loaded.display_title(), "hello world");
    assert_eq!(loaded.summary, "remember this");
}

#[tokio::test]
async fn delete_is_scoped_to_owner_and_clear_all_resets_current() {
    let db = common::test_db().await;
    let mine = db::create_thread(&db.pool, 10).await.unwrap();
    let theirs = db::create_thread(&db.pool, 11).await.unwrap();

    assert!(!db::delete_thread(&db.pool, 10, theirs.id).await.unwrap());
    assert!(db::get_thread(&db.pool, theirs.id).await.is_ok());

    assert!(db::delete_thread(&db.pool, 10, mine.id).await.unwrap());
    assert!(db::get_thread(&db.pool, mine.id).await.is_err());
    assert_eq!(
        db::get_current_thread_id(&db.pool, 10).await.unwrap(),
        None,
        "ON DELETE SET NULL clears the current thread"
    );

    let _ = db::create_thread(&db.pool, 11).await.unwrap();
    let n = db::delete_all_threads(&db.pool, 11).await.unwrap();
    assert_eq!(n, 2);
    assert!(db::list_threads(&db.pool, 11).await.unwrap().is_empty());
    assert_eq!(db::get_current_thread_id(&db.pool, 11).await.unwrap(), None);
}

#[tokio::test]
async fn skills_can_be_toggled_per_user() {
    let db = common::test_db().await;
    db::set_skill(&db.pool, 3, "web", true).await.unwrap();
    db::set_skill(&db.pool, 4, "web", true).await.unwrap();
    db::set_skill(&db.pool, 3, "web", false).await.unwrap();

    let enabled_3 = db::enabled_skills(&db.pool, 3).await.unwrap();
    let enabled_4 = db::enabled_skills(&db.pool, 4).await.unwrap();
    assert!(enabled_3.is_empty());
    assert!(enabled_4.contains("web"));
}

#[tokio::test]
async fn set_current_thread_can_clear_selection() {
    let db = common::test_db().await;
    let thread = db::create_thread(&db.pool, 2).await.unwrap();
    db::set_current_thread(&db.pool, 2, None).await.unwrap();
    assert_eq!(db::get_current_thread_id(&db.pool, 2).await.unwrap(), None);
    db::set_current_thread(&db.pool, 2, Some(thread.id))
        .await
        .unwrap();
    assert_eq!(
        db::get_current_thread_id(&db.pool, 2).await.unwrap(),
        Some(thread.id)
    );
}

#[tokio::test]
async fn missing_thread_is_an_error() {
    let db = common::test_db().await;
    assert!(db::get_thread(&db.pool, 999).await.is_err());
}
