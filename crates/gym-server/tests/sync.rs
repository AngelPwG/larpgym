use chrono::NaiveDate;
use gym_core::*;
use gym_server::router;
use gym_storage::{Role, Store};
use gym_sync::SyncClient;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use uuid::Uuid;
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(path: &Path) -> Server {
    let store = Store::open(path, Role::Server).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router(store, TOKEN)).await.unwrap();
    });
    Server { url, task }
}
fn client(path: &Path) -> Store {
    Store::open(path, Role::Client).unwrap()
}
fn date(n: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, n).unwrap()
}
fn weight(n: u32, grams: u32) -> Document {
    let mut d = Document::day(date(n));
    if let Entity::Day(day) = &mut d.entity {
        day.weight = Some(grams);
    }
    d
}
fn path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}
async fn sync(server: &Server, p: &Path) {
    SyncClient::new(&server.url, TOKEN.into())
        .unwrap()
        .synchronize(p)
        .await
        .unwrap();
}

#[tokio::test]
async fn offline_restart_two_clients_update_delete_and_lost_response() {
    let dir = TempDir::new().unwrap();
    let a = path(&dir, "a.sqlite");
    let b = path(&dir, "b.sqlite");
    let doc = weight(1, 72000);
    let device = {
        let mut s = client(&a);
        s.save(doc.clone()).unwrap();
        s.device_id().unwrap()
    };
    assert_eq!(client(&a).device_id().unwrap(), device);
    assert_eq!(client(&a).snapshot().unwrap().pending, 1);
    assert_eq!(client(&a).document(doc.id).unwrap(), Some(doc.clone()));
    let unavailable = SyncClient::new("http://127.0.0.1:1", TOKEN.into()).unwrap();
    assert!(unavailable.synchronize(&a).await.is_err());
    assert_eq!(client(&a).snapshot().unwrap().pending, 1);
    let server = server(&path(&dir, "server.sqlite")).await;
    let http = SyncClient::new(&server.url, TOKEN.into()).unwrap();
    let op = client(&a).prepare_next().unwrap().unwrap();
    let first = http.push(&op).await.unwrap(); // Simulate process death after send, before acknowledging.
    drop(client(&a));
    let retried = client(&a).prepare_next().unwrap().unwrap();
    assert_eq!(op, retried);
    let second = http.push(&retried).await.unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert_eq!(client(&b).documents().unwrap(), vec![doc.clone()]);
    assert_eq!(http.pull(0).await.unwrap().changes.len(), 1);
    let changed = weight(1, 71500);
    client(&a).save(changed.clone()).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert_eq!(client(&b).document(doc.id).unwrap(), Some(changed.clone()));
    let mut deleted = changed;
    deleted.deleted = true;
    client(&a).save(deleted.clone()).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert!(client(&b).document(doc.id).unwrap().unwrap().deleted);
    let mut resurrected = deleted;
    resurrected.deleted = false;
    client(&b).save(resurrected.clone()).unwrap();
    sync(&server, &b).await;
    sync(&server, &a).await;
    assert_eq!(client(&a).document(doc.id).unwrap(), Some(resurrected));
}

#[tokio::test]
async fn concurrent_edits_keep_both_and_resolution_checks_latest_version() {
    let dir = TempDir::new().unwrap();
    let a = path(&dir, "a");
    let b = path(&dir, "b");
    let server = server(&path(&dir, "server")).await;
    client(&a).save(weight(2, 70000)).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    client(&a).save(weight(2, 71000)).unwrap();
    client(&b).save(weight(2, 72000)).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    let conflict = client(&b).conflicts().unwrap().remove(0);
    assert_eq!(conflict.local, weight(2, 72000));
    assert_eq!(
        conflict.current.as_ref().unwrap().document,
        weight(2, 71000)
    );
    // A changes again after B opened the conflict. Resolution must fail against stale displayed state.
    client(&a).save(weight(2, 73000)).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert!(client(&b).resolve(&conflict, true).is_err());
    let latest = client(&b).conflicts().unwrap().remove(0);
    client(&b).resolve(&latest, true).unwrap();
    // Another server edit between local resolution and its push is detected by the server too.
    client(&a).save(weight(2, 74000)).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert_eq!(client(&b).conflicts().unwrap().len(), 1);
    let latest = client(&b).conflicts().unwrap().remove(0);
    client(&b).resolve(&latest, true).unwrap();
    sync(&server, &b).await;
    sync(&server, &a).await;
    assert_eq!(
        client(&a).document(day_id(date(2))).unwrap(),
        Some(weight(2, 72000))
    );
    assert!(client(&b).conflicts().unwrap().is_empty());
}

#[tokio::test]
async fn daily_unique_weight_and_habit_conflicts_can_choose_server() {
    let dir = TempDir::new().unwrap();
    let a = path(&dir, "a");
    let b = path(&dir, "b");
    let server = server(&path(&dir, "server")).await;
    let habit = initial_catalogue().pop().unwrap();
    client(&a).save(habit.clone()).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    let mut da = weight(3, 70000);
    if let Entity::Day(d) = &mut da.entity {
        d.habits.insert(habit.id, HabitState::Done);
    }
    let mut db = weight(3, 71000);
    if let Entity::Day(d) = &mut db.entity {
        d.habits.insert(habit.id, HabitState::Skipped);
    }
    client(&a).save(da.clone()).unwrap();
    client(&b).save(db).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert_eq!(client(&b).conflicts().unwrap().len(), 1);
    let c = client(&b).conflicts().unwrap().remove(0);
    client(&b).resolve(&c, false).unwrap();
    sync(&server, &b).await;
    assert_eq!(client(&b).document(da.id).unwrap(), Some(da));
    assert_eq!(
        client(&b)
            .documents()
            .unwrap()
            .iter()
            .filter(|d| matches!(d.entity, Entity::Day(_)))
            .count(),
        1
    );
    let mut bad = weight(3, 70000);
    bad.id = Uuid::new_v4();
    assert!(client(&b).save(bad).is_err());
}

#[tokio::test]
async fn multiple_pages_order_dependencies_completion_and_zero_load() {
    let dir = TempDir::new().unwrap();
    let a = path(&dir, "a");
    let b = path(&dir, "b");
    let server = server(&path(&dir, "server")).await;
    let exercises = initial_catalogue();
    let e = exercises[0].clone();
    client(&a).save(e.clone()).unwrap();
    let mut doc = Document::day(date(4));
    if let Entity::Day(d) = &mut doc.entity {
        d.sessions.push(Session {
            id: Uuid::new_v4(),
            name: "Sesión".into(),
            notes: "".into(),
            status: SessionStatus::InProgress,
            exercises: vec![SessionExercise {
                id: Uuid::new_v4(),
                exercise_id: e.id,
                name: "Sentadilla".into(),
                target_sets: None,
                target_reps: None,
                sets: vec![Set {
                    id: Uuid::new_v4(),
                    load: 0,
                    reps: 10,
                }],
            }],
        });
        d.complete_session(d.sessions[0].id).unwrap();
    }
    client(&a).save(doc.clone()).unwrap();
    for i in 0..205 {
        let d = Document {
            id: Uuid::new_v4(),
            deleted: false,
            entity: Entity::Exercise(Exercise {
                name: format!("Ejercicio {i}"),
                archived: false,
            }),
        };
        client(&a).save(d).unwrap();
    }
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert_eq!(client(&b).documents().unwrap().len(), 207);
    assert_eq!(client(&b).cursor().unwrap(), 207);
    assert_eq!(client(&b).document(doc.id).unwrap(), Some(doc));
    let h = SyncClient::new(&server.url, TOKEN.into()).unwrap();
    let p1 = h.pull(0).await.unwrap();
    assert_eq!(p1.changes.len(), 100);
    assert!(p1.has_more);
    let p2 = h.pull(p1.next_cursor).await.unwrap();
    assert_eq!(p2.changes.len(), 100);
    assert!(p2.has_more);
    let p3 = h.pull(p2.next_cursor).await.unwrap();
    assert_eq!(p3.changes.len(), 7);
    assert!(!p3.has_more);
}

#[tokio::test]
async fn frozen_outbox_preserves_later_edits_and_conflicts_survive_restart() {
    let dir = TempDir::new().unwrap();
    let a = path(&dir, "a");
    let b = path(&dir, "b");
    let server = server(&path(&dir, "server")).await;
    client(&a).save(weight(5, 70000)).unwrap();
    let op = client(&a).prepare_next().unwrap().unwrap();
    let result = SyncClient::new(&server.url, TOKEN.into())
        .unwrap()
        .push(&op)
        .await
        .unwrap();
    // Confirm additional changes while the response is in flight.
    client(&a).save(weight(5, 71000)).unwrap();
    client(&a).save(weight(5, 72000)).unwrap();
    client(&a).acknowledge(&op, &result).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert_eq!(
        client(&b).document(op.document.id).unwrap(),
        Some(weight(5, 72000))
    );
    assert_eq!(client(&a).snapshot().unwrap().pending, 0);
    let mut c = client(&a);
    let lock = c.sync_lock().unwrap();
    assert!(c.sync_lock().is_err());
    drop(lock);
    assert!(c.sync_lock().is_ok());
    c.save(weight(5, 73000)).unwrap();
    client(&b).save(weight(5, 74000)).unwrap();
    sync(&server, &b).await;
    sync(&server, &a).await;
    drop(c);
    assert_eq!(client(&a).snapshot().unwrap().conflicts.len(), 1);
    assert_eq!(
        client(&a).document(op.document.id).unwrap(),
        Some(weight(5, 73000))
    );
}

#[tokio::test]
async fn authentication_limits_invalid_relations_and_operation_reuse() {
    let dir = TempDir::new().unwrap();
    let server = server(&path(&dir, "server")).await;
    let http = reqwest::Client::new();
    assert_eq!(
        http.get(format!("{}/healthz", server.url))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    for token in [None, Some("incorrecto")] {
        let mut r = http.get(format!("{}/api/v1/changes", server.url));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        assert_eq!(r.send().await.unwrap().status(), 401);
    }
    assert_eq!(
        http.post(format!("{}/api/v1/push", server.url))
            .json(&PushRequest { operations: vec![] })
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.get(format!("{}/api/v1/changes?limit=101", server.url))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        http.post(format!("{}/api/v1/push", server.url))
            .bearer_auth(TOKEN)
            .header("content-type", "application/json")
            .body("x".repeat(1024 * 1024 + 1))
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    let id = Uuid::new_v4();
    let op = Operation {
        id: Uuid::new_v4(),
        device_id: Uuid::new_v4(),
        base_version: 0,
        strict_version: false,
        document: Document {
            id,
            deleted: false,
            entity: Entity::Routine(Routine {
                name: "Inválida".into(),
                archived: false,
                entries: vec![RoutineEntry {
                    exercise_id: Uuid::new_v4(),
                    target_sets: None,
                    target_reps: None,
                }],
            }),
        },
    };
    let sync = SyncClient::new(&server.url, TOKEN.into()).unwrap();
    assert!(matches!(
        sync.push(&op).await.unwrap(),
        Outcome::Conflict { .. }
    ));
    let mut changed = op;
    changed.base_version = 9;
    assert!(sync.push(&changed).await.is_err());
    assert!(sync.pull(1).await.is_err());
}

#[tokio::test]
async fn choosing_server_also_checks_version_after_resolution() {
    let dir = TempDir::new().unwrap();
    let a = path(&dir, "a");
    let b = path(&dir, "b");
    let server = server(&path(&dir, "server")).await;
    client(&a).save(weight(6, 70000)).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    client(&a).save(weight(6, 71000)).unwrap();
    client(&b).save(weight(6, 72000)).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    let conflict = client(&b).conflicts().unwrap().remove(0);
    client(&b).resolve(&conflict, false).unwrap();
    client(&a).save(weight(6, 73000)).unwrap();
    sync(&server, &a).await;
    sync(&server, &b).await;
    assert_eq!(client(&b).conflicts().unwrap().len(), 1);
    let latest = client(&b).conflicts().unwrap().remove(0);
    client(&b).resolve(&latest, false).unwrap();
    sync(&server, &b).await;
    assert!(client(&b).conflicts().unwrap().is_empty());
    assert_eq!(
        client(&b).document(day_id(date(6))).unwrap(),
        Some(weight(6, 73000))
    );
}

#[tokio::test]
async fn fresh_device_adopts_modified_catalogue_but_keeps_real_offline_edits() {
    let dir = TempDir::new().unwrap();
    let a = path(&dir, "a");
    let b = path(&dir, "b");
    let c = path(&dir, "c");
    let server = server(&path(&dir, "server")).await;
    client(&a).seed_catalogue().unwrap();
    sync(&server, &a).await;
    let mut edited = initial_catalogue().remove(0);
    if let Entity::Exercise(e) = &mut edited.entity {
        e.name = "Sentadilla con barra".into();
        e.archived = true;
    }
    client(&a).save(edited.clone()).unwrap();
    sync(&server, &a).await;
    client(&b).seed_catalogue().unwrap();
    sync(&server, &b).await;
    assert!(client(&b).conflicts().unwrap().is_empty());
    assert_eq!(client(&b).snapshot().unwrap().pending, 0);
    assert_eq!(
        client(&b).document(edited.id).unwrap(),
        Some(edited.clone())
    );
    client(&c).seed_catalogue().unwrap();
    let mut offline = edited.clone();
    if let Entity::Exercise(e) = &mut offline.entity {
        e.name = "Mi propuesta offline".into();
    }
    client(&c).save(offline.clone()).unwrap();
    sync(&server, &c).await;
    assert_eq!(client(&c).conflicts().unwrap().len(), 1);
    assert_eq!(client(&c).document(edited.id).unwrap(), Some(offline));
}

#[tokio::test]
async fn server_backup_restores_journal_and_idempotency_receipts() {
    let dir = TempDir::new().unwrap();
    let a = path(&dir, "a");
    let b = path(&dir, "b");
    let original = path(&dir, "server");
    let running = server(&original).await;
    client(&a).save(weight(7, 70000)).unwrap();
    sync(&running, &a).await;
    client(&a).save(weight(8, 71000)).unwrap();
    let op = client(&a).prepare_next().unwrap().unwrap();
    let before = SyncClient::new(&running.url, TOKEN.into())
        .unwrap()
        .push(&op)
        .await
        .unwrap();
    let backup = path(&dir, "server-backup");
    Store::open(&original, Role::Server)
        .unwrap()
        .backup(&backup)
        .unwrap();
    drop(running);
    let restored = server(&backup).await;
    let after = SyncClient::new(&restored.url, TOKEN.into())
        .unwrap()
        .push(&op)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    sync(&restored, &a).await;
    sync(&restored, &b).await;
    assert_eq!(client(&a).snapshot().unwrap().pending, 0);
    assert_eq!(client(&b).documents().unwrap().len(), 2);
    assert_eq!(client(&b).cursor().unwrap(), 2);
}
