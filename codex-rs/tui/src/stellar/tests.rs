use super::StellarController;
use super::StellarView;
use codex_core::stellar::StellarPersona;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use insta::assert_snapshot;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn send(ctrl: &mut StellarController, code: KeyCode, modifiers: KeyModifiers) {
    let event = KeyEvent::new(code, modifiers);
    let _ = ctrl.handle_key_event(event);
}

fn activate_canvas(ctrl: &mut StellarController) {
    send(ctrl, KeyCode::Char('i'), KeyModifiers::CONTROL);
}

fn seed_insight(ctrl: &mut StellarController) {
    ctrl.set_field_text("Investigate APDEX drift");
    send(ctrl, KeyCode::Char('o'), KeyModifiers::CONTROL);
    send(ctrl, KeyCode::Char('i'), KeyModifiers::NONE);
    send(ctrl, KeyCode::Enter, KeyModifiers::CONTROL);
}

#[test]
fn stellar_view_wide_snapshot() {
    let mut controller = StellarController::new(StellarPersona::Operator);
    activate_canvas(&mut controller);
    seed_insight(&mut controller);
    controller.sync_layout(140);
    controller.take_events();

    let snapshot = controller.snapshot();
    let mut terminal = Terminal::new(TestBackend::new(140, 20)).expect("terminal");
    terminal
        .draw(|f| {
            f.render_widget(StellarView::new(&snapshot), f.area());
        })
        .expect("draw");

    assert_snapshot!("stellar_view_wide", terminal.backend());
}

#[test]
fn stellar_view_compact_snapshot() {
    let mut controller = StellarController::new(StellarPersona::Operator);
    activate_canvas(&mut controller);
    seed_insight(&mut controller);
    controller.sync_layout(90);
    controller.take_events();

    let snapshot = controller.snapshot();
    let mut terminal = Terminal::new(TestBackend::new(90, 22)).expect("terminal");
    terminal
        .draw(|f| {
            f.render_widget(StellarView::new(&snapshot), f.area());
        })
        .expect("draw");

    assert_snapshot!("stellar_view_compact", terminal.backend());
}

#[test]
fn ctrl_o_toggles_telemetry_overlay() {
    let mut controller = StellarController::new(StellarPersona::Operator);
    activate_canvas(&mut controller);

    assert!(!controller.snapshot().telemetry_visible);

    send(&mut controller, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert!(controller.snapshot().telemetry_visible);

    send(&mut controller, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert!(!controller.snapshot().telemetry_visible);
}
