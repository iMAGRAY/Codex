use std::sync::Arc;

use eyre::Result;

use crate::backend::{AppServiceHandle, CodexBackend};
use codex_core::config::{Config, ConfigOverrides};
use codex_core::{AuthManager, ConversationManager};

pub fn init_service_handle() -> Result<AppServiceHandle> {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
    );

    let config = Config::load_with_cli_overrides(Vec::new(), gui_overrides())?;
    let auth_manager = AuthManager::shared(config.codex_home.clone());
    let conversation_manager = Arc::new(ConversationManager::new(auth_manager));

    let backend = CodexBackend::new(runtime, conversation_manager, config);
    Ok(AppServiceHandle::new(Arc::new(backend)))
}

fn gui_overrides() -> ConfigOverrides {
    ConfigOverrides {
        model: None,
        review_model: None,
        cwd: None,
        approval_policy: None,
        sandbox_mode: None,
        model_provider: None,
        config_profile: None,
        codex_linux_sandbox_exe: None,
        base_instructions: None,
        include_plan_tool: Some(true),
        include_apply_patch_tool: None,
        include_view_image_tool: Some(true),
        show_raw_agent_reasoning: Some(false),
        tools_web_search_request: Some(true),
    }
}
