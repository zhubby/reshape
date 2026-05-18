use reshape_core::prompt::system_prompt;

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
fn prompt_requires_html_artifacts_and_index_links() {
    let prompt = system_prompt();

    assert!(prompt.contains("Every generated page artifact must be an HTML file"));
    assert!(prompt.contains("Do not return raw HTML as the final answer"));
    assert!(prompt.contains("write_file"));
    assert!(prompt.contains("index.html"));
    assert!(prompt.contains("relative link"));
    assert!(prompt.contains("No generated HTML page may be orphaned"));
}

#[test]
fn prompt_requires_wiki_style_workspace_structure() {
    let prompt = system_prompt();

    assert!(prompt.contains("wiki-like site"));
    assert!(prompt.contains("map_workspace"));
    assert!(prompt.contains("hub"));
    assert!(prompt.contains("pages/"));
    assert!(prompt.contains("assets/"));
    assert!(prompt.contains("kebab-case"));
}

#[test]
fn prompt_requires_default_layout_and_stylesheet_contract() {
    let prompt = system_prompt();

    assert!(prompt.contains("assets/site.css"));
    assert!(prompt.contains(".masthead"));
    assert!(prompt.contains(".hero"));
    assert!(prompt.contains(".prose"));
    assert!(prompt.contains("--measure"));
    assert!(prompt.contains("inline"));
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
