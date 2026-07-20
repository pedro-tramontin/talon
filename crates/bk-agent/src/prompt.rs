//! System prompt template for the agent.

use crate::tools::ToolSchema;
use bk_core::ProjectId;

/// Render the system prompt that is sent to the LLM on every run.
///
/// `tool_list` contains only the tools in the current `allowed_tools`
/// set, so the LLM cannot be instructed to use a tool that the agent
/// will refuse to call.
pub fn render_system_prompt(
    project_name: &str,
    project_id: ProjectId,
    target_host: &str,
    user_goal: &str,
    tool_list: &[ToolSchema],
) -> String {
    let names: Vec<String> = tool_list.iter().map(|t| t.name.to_string()).collect();
    format!(
        "You are an agent driving Talon, a web-security toolkit. You have \
         access to the following tools: {tool_list}.

Current project: {project_name} (id: {project_id}).
Target host: {target_host}.

Your job is to {user_goal}. When you're done, respond with a summary \
         of what you did and what you found. Be concise.

Rules:
- Only use tools that are in your allowed list.
- If a tool returns an error, try a different approach — don't loop \
  on the same error.
- Prefer reading (list, get, search) before writing (insert, update, \
  delete) — know what you're changing before you change it.
- When you call a tool, the result is added to your conversation \
  history; you don't need to repeat the user's question.",
        tool_list = names.join(", "),
        project_name = project_name,
        project_id = project_id,
        target_host = target_host,
        user_goal = user_goal,
    )
}
