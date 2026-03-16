use shared_protocol::{MemoryRecord, UserAccount, UserChannelBinding};
use storage::{GatewayRepository, SqliteRepository};

#[test]
fn tenant_isolation_blocks_cross_tenant_memory_reads() {
    let repo = SqliteRepository::in_memory().expect("repo");
    repo.append_memory(&MemoryRecord {
        tenant_id: "tenant-a".to_owned(),
        user_id: "user-1".to_owned(),
        session_id: "session-a".to_owned(),
        content: "memory-a".to_owned(),
    })
    .expect("insert");

    let own = repo.list_memory("tenant-a", "user-1", "session-a").expect("list own");
    assert_eq!(own.len(), 1);

    let isolated = repo.list_memory("tenant-b", "user-1", "session-a").expect("list other");
    assert!(isolated.is_empty());
}

#[test]
fn user_channel_bindings_resolve_independently() {
    let repo = SqliteRepository::in_memory().expect("repo");
    repo.upsert_user(&UserAccount {
        tenant_id: "tenant-a".to_owned(),
        user_id: "alice".to_owned(),
        display_name: "Alice".to_owned(),
    })
    .expect("alice");
    repo.upsert_user(&UserAccount {
        tenant_id: "tenant-a".to_owned(),
        user_id: "bob".to_owned(),
        display_name: "Bob".to_owned(),
    })
    .expect("bob");
    repo.upsert_channel_binding(&UserChannelBinding {
        tenant_id: "tenant-a".to_owned(),
        user_id: "alice".to_owned(),
        channel_name: "discord".to_owned(),
        external_user: "disc-alice".to_owned(),
        credentials_json: "{\"token\":\"a\"}".to_owned(),
    })
    .expect("alice binding");
    repo.upsert_channel_binding(&UserChannelBinding {
        tenant_id: "tenant-a".to_owned(),
        user_id: "bob".to_owned(),
        channel_name: "discord".to_owned(),
        external_user: "disc-bob".to_owned(),
        credentials_json: "{\"token\":\"b\"}".to_owned(),
    })
    .expect("bob binding");

    let alice = repo
        .resolve_channel_user("tenant-a", "discord", "disc-alice")
        .expect("resolve alice")
        .expect("alice exists");
    let bob = repo
        .resolve_channel_user("tenant-a", "discord", "disc-bob")
        .expect("resolve bob")
        .expect("bob exists");
    assert_eq!(alice.user_id, "alice");
    assert_eq!(bob.user_id, "bob");
}
