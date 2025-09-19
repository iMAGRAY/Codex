use codex_gui::{AppServiceHandle, PromptPayload, SessionRequest};

#[test]
fn smoke_session_flow() {
    let handle = AppServiceHandle::mock();
    let sessions = handle.list_sessions().expect("sessions list");
    assert!(sessions.len() >= 1, "ожидаем хотя бы одну демо-сессию");

    let descriptor = handle
        .spawn_session(SessionRequest::default())
        .expect("spawn session");
    handle
        .send_prompt(
            &descriptor.id,
            PromptPayload {
                text: "ping".into(),
            },
        )
        .expect("send prompt");
    let history = handle.list_history(&descriptor.id).expect("history");
    assert!(
        history
            .iter()
            .any(|m| matches!(m.role, codex_gui::MessageRole::Assistant))
    );
}
