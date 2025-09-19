mod backend;
mod runtime;
mod ui;

pub use backend::AppServiceHandle;
pub use backend::BackendEvent;
pub use backend::HistoryEvent;
pub use backend::HistoryItem;
pub use backend::MessageRole;
pub use backend::PromptPayload;
pub use backend::SeedMessage;
pub use backend::SessionDescriptor;
pub use backend::SessionId;
pub use backend::SessionRequest;
pub use backend::SessionStream;
pub use backend::StatusEvent;
pub use ui::DesktopShell;

use eframe::egui;
use eyre::Result;

pub fn bootstrap(handle: AppServiceHandle) -> Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Codex Desktop")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([960.0, 600.0]),
        follow_system_theme: false,
        default_theme: eframe::Theme::Light,
        ..Default::default()
    };

    let app_handle = handle.clone();
    eframe::run_native(
        "Codex Desktop",
        native_options,
        Box::new(move |cc| {
            let app = DesktopShell::new(&cc.egui_ctx, app_handle.clone());
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Box::new(app))
        }),
    )
    .map_err(|err| eyre::eyre!("Не удалось запустить десктопный UI: {err}"))
}

pub fn default_service_handle() -> Result<AppServiceHandle> {
    runtime::init_service_handle()
}
