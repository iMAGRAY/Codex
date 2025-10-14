use crate::RolloutRecorder;
use crate::exec_command::ExecFlowRegistry;
use crate::exec_command::ExecSessionManager;
use crate::executor::Executor;
use crate::mcp_connection_manager::McpConnectionManager;
use crate::unified_exec::UnifiedExecSessionManager;
use crate::user_notification::UserNotifier;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct SessionServices {
    pub(crate) mcp_connection_manager: McpConnectionManager,
    pub(crate) exec_flow_registry: Arc<ExecFlowRegistry>,
    pub(crate) unified_exec_manager: UnifiedExecSessionManager,
    pub(crate) notifier: UserNotifier,
    pub(crate) rollout: Mutex<Option<RolloutRecorder>>,
    pub(crate) user_shell: crate::shell::Shell,
    pub(crate) show_raw_agent_reasoning: bool,
    pub(crate) executor: Executor,
}

impl SessionServices {
    pub(crate) fn exec_flow_manager(&self) -> ExecSessionManager {
        self.exec_flow_registry.session_manager()
    }
}
