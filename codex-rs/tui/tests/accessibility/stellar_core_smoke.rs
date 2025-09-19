use codex_core::stellar::KernelSnapshot;
use codex_core::stellar::StellarPersona;
use codex_tui::stellar::StellarController;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;

fn snapshot_for(persona: StellarPersona) -> KernelSnapshot {
    let mut controller = StellarController::new(persona);
    let _ = controller.handle_key_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
    controller.set_field_text("Screen reader baseline insight");
    let _ = controller.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
    controller.sync_layout(120);
    controller.snapshot()
}

#[test]
fn golden_path_and_hints_present() {
    let snapshot = snapshot_for(StellarPersona::AssistiveBridge);
    assert!(snapshot.assistive_mode, "assistive mode should be active");
    assert!(
        snapshot
            .golden_path
            .iter()
            .any(|hint| hint.description.to_lowercase().contains("telemetry"))
    );
    assert!(
        snapshot
            .command_log
            .iter()
            .any(|entry| entry.contains("Screen reader baseline insight"))
    );
    assert!(
        snapshot
            .status_messages
            .iter()
            .any(|msg| msg.to_lowercase().contains("insight submitted"))
    );
    assert!(
        snapshot
            .golden_path
            .iter()
            .any(|hint| hint.shortcut.as_deref() == Some("Ctrl+Enter"))
    );
}
