use reshape::prompt::system_prompt;

#[test]
fn prompt_requires_workspace_file_workflow_and_completion_signal() {
    let prompt = system_prompt();

    assert!(prompt.contains("HTML"));
    assert!(prompt.contains("CSS"));
    assert!(prompt.contains("JavaScript"));
    assert!(prompt.contains("complete_task"));
    assert!(prompt.contains("configured workspace"));
}

#[test]
fn prompt_preserves_single_session_boundary() {
    let prompt = system_prompt();

    assert!(prompt.contains("local:main"));
    assert!(!prompt.contains("multi-session"));
    assert!(!prompt.contains("multiple sessions"));
}

#[test]
fn prompt_does_not_claim_browser_render_without_confirmation() {
    let prompt = system_prompt();

    assert!(prompt.contains("Do not claim a page is visible in the browser"));
    assert!(prompt.contains("CDP"));
}
