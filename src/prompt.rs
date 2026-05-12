pub fn system_prompt() -> String {
    include_str!("../docs/agent-system-prompt.md").to_string()
}
