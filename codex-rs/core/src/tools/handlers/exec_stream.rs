use async_trait::async_trait;
use serde::Deserialize;

use crate::exec_command::EXEC_COMMAND_TOOL_NAME;
use crate::exec_command::EXEC_CONTROL_TOOL_NAME;
use crate::exec_command::ExecCommandParams;
use crate::exec_command::ExecControlParams;
use crate::exec_command::GET_SESSION_EVENTS_TOOL_NAME;
use crate::exec_command::LIST_EXEC_SESSIONS_TOOL_NAME;
use crate::exec_command::SessionEventsParams;
use crate::exec_command::SessionLifecycle;
use crate::exec_command::WRITE_STDIN_TOOL_NAME;
use crate::exec_command::WriteStdinParams;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct ExecStreamHandler;

#[async_trait]
impl ToolHandler for ExecStreamHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            tool_name,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "exec_stream handler received unsupported payload".to_string(),
                ));
            }
        };

        let content = match tool_name.as_str() {
            EXEC_COMMAND_TOOL_NAME => {
                let params: ExecCommandParams = serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse function arguments: {e:?}"
                    ))
                })?;
                session.handle_exec_command_tool(params).await?
            }
            WRITE_STDIN_TOOL_NAME => {
                let params: WriteStdinParams = serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse function arguments: {e:?}"
                    ))
                })?;
                session.handle_write_stdin_tool(params).await?
            }
            EXEC_CONTROL_TOOL_NAME => {
                let params: ExecControlParams = serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse function arguments: {e:?}"
                    ))
                })?;
                session.handle_exec_control_tool(params).await?
            }
            LIST_EXEC_SESSIONS_TOOL_NAME => {
                #[derive(Debug, Default, Deserialize)]
                struct ListSessionsArgs {
                    #[serde(default)]
                    state: Option<String>,
                    #[serde(default)]
                    limit: Option<usize>,
                    #[serde(default)]
                    since_ms: Option<u64>,
                }

                let args: ListSessionsArgs = if arguments.trim().is_empty() {
                    ListSessionsArgs::default()
                } else {
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse function arguments: {e:?}"
                        ))
                    })?
                };

                let state_filter = args
                    .state
                    .as_deref()
                    .map(|state| match state {
                        "running" => Ok(SessionLifecycle::Running),
                        "grace" => Ok(SessionLifecycle::Grace),
                        "terminated" => Ok(SessionLifecycle::Terminated),
                        other => Err(FunctionCallError::RespondToModel(format!(
                            "invalid state filter: {other}"
                        ))),
                    })
                    .transpose()?;

                session
                    .handle_list_exec_sessions_tool(state_filter, args.limit, args.since_ms)
                    .await?
            }
            GET_SESSION_EVENTS_TOOL_NAME => {
                let params: SessionEventsParams =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse function arguments: {e:?}"
                        ))
                    })?;
                session.handle_get_session_events_tool(params).await?
            }
            _ => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "exec_stream handler does not support tool {tool_name}"
                )));
            }
        };

        Ok(ToolOutput::Function {
            content,
            success: Some(true),
        })
    }
}
