use reshape_core::protocol::{DEFAULT_SESSION_KEY, Envelope, InputEvent, InputSource, OutputEvent};
use reshape_core::session::store::{FileSessionStore, InMemorySessionStore, SessionStore};
use reshape_core::session::{Session, TurnState};

#[test]
fn envelope_defaults_to_single_local_session() {
    let envelope = Envelope::new(InputEvent::UserText {
        text: "build a landing page".to_string(),
        source: InputSource::Test,
    });

    assert_eq!(envelope.header.session_key, DEFAULT_SESSION_KEY);
    assert_eq!(envelope.header.attempt, 1);
    assert_eq!(envelope.header.schema_version, "1.0");
}

#[test]
fn turn_state_accepts_expected_runtime_path() {
    let state = TurnState::Received
        .transition_to(TurnState::Validating)
        .unwrap()
        .transition_to(TurnState::Executing)
        .unwrap()
        .transition_to(TurnState::Publishing)
        .unwrap()
        .transition_to(TurnState::Completed)
        .unwrap();

    assert_eq!(state, TurnState::Completed);
}

#[test]
fn turn_state_rejects_invalid_transition() {
    let error = TurnState::Received
        .transition_to(TurnState::Completed)
        .unwrap_err();

    assert!(error.to_string().contains("invalid state transition"));
}

#[tokio::test]
async fn session_store_persists_single_session_history() {
    let store = InMemorySessionStore::default();
    let mut session = Session::default();

    session.record_input(InputEvent::UserText {
        text: "make it blue".to_string(),
        source: InputSource::Test,
    });
    session.record_output(OutputEvent::Completed {
        summary: "updated page".to_string(),
    });

    store.save(session).await.unwrap();
    let loaded = store.load().await.unwrap();

    assert_eq!(loaded.session_key, DEFAULT_SESSION_KEY);
    assert_eq!(loaded.turn_index, 1);
    assert_eq!(loaded.history.len(), 2);
}

#[tokio::test]
async fn file_session_store_loads_default_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileSessionStore::new(dir.path().join("session.json"));

    let loaded = store.load().await.unwrap();

    assert_eq!(loaded, Session::default());
}

#[tokio::test]
async fn file_session_store_persists_single_session_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("session.json");
    let store = FileSessionStore::new(path.clone());
    let mut session = Session::default();

    session.record_input(InputEvent::UserText {
        text: "make it persistent".to_string(),
        source: InputSource::WebSocket,
    });
    session.record_output(OutputEvent::Completed {
        summary: "persistent page updated".to_string(),
    });

    store.save(session.clone()).await.unwrap();
    let reloaded = FileSessionStore::new(path).load().await.unwrap();

    assert_eq!(reloaded, session);
}

#[tokio::test]
async fn file_session_store_reports_corrupt_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    tokio::fs::write(&path, "{not-json").await.unwrap();
    let store = FileSessionStore::new(path);

    let error = store.load().await.unwrap_err();

    assert!(error.to_string().contains("failed to parse session store"));
}
