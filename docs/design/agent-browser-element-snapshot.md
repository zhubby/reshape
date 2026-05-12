# Agent Browser — Element Resolution & Accessibility Snapshot

This document covers the two core modules that power the agent-browser's ability to
identify, reference, and describe page elements: **element.rs** (resolution & property
queries) and **snapshot.rs** (accessibility tree snapshot). Together they form the bridge
between CDP (Chrome DevTools Protocol) and the LLM, turning raw DOM/AX data into
stable, human-readable references the agent can act on.

---

## 1. element.rs — Element Resolution & Property Queries

The element module provides a ref-based addressing system and a suite of async functions
that resolve refs and CSS selectors into CDP coordinates (center points, object IDs) and
query element properties via JavaScript evaluation.

### 1.1 RefEntry

```rust
pub struct RefEntry {
    pub backend_node_id: Option<i64>,
    pub role: String,
    pub name: String,
    pub nth: Option<usize>,
    pub selector: Option<String>,
    pub frame_id: Option<String>,
}
```

Each `RefEntry` stores the metadata needed to re-locate an element after the initial
snapshot. The fields serve distinct purposes:

| Field | Purpose |
|-------|---------|
| `backend_node_id` | CDP `backendDOMNodeId` — the primary fast-path identifier for `DOM.getBoxModel` and `DOM.resolveNode`. May become stale when the page mutates. |
| `role` | ARIA role string (e.g. `"button"`, `"link"`, `"textbox"`). Used as the fallback re-query key when `backend_node_id` is stale. |
| `name` | ARIA accessible name (computed label). Paired with `role` for stale-node fallback. |
| `nth` | Disambiguation index when multiple nodes share the same `role`+`name` combination (e.g. two `"button"` elements both named `"Submit"`). `None` when the combination is unique. |
| `selector` | CSS or XPath selector string. Populated only by `add_selector`; used when the ref was created from a selector rather than an AX tree walk. |
| `frame_id` | CDP frame ID for elements inside iframes. Determines which CDP session is used for cross-origin iframe routing. |

### 1.2 RefMap

```rust
pub struct RefMap {
    map: HashMap<String, RefEntry>,
    next_ref: usize,
}
```

`RefMap` is the central registry that maps short ref identifiers (e.g. `"e5"`) to
`RefEntry` records. It is populated during `take_snapshot` and consumed by all
element-resolution and property-query functions.

#### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| **`new`** | `() → Self` | Creates an empty map with `next_ref = 1`. |
| **`add`** | `(ref_id, backend_node_id, role, name, nth)` | Convenience wrapper around `add_with_frame` with `frame_id = None`. |
| **`add_with_frame`** | `(ref_id, backend_node_id, role, name, nth, frame_id)` | Inserts a full `RefEntry` including iframe frame association. Used during snapshot tree construction for AX-derived refs. |
| **`add_selector`** | `(ref_id, selector, role, name, nth)` | Inserts a `RefEntry` with `backend_node_id = None` and `selector` populated. Used when refs originate from CSS/XPath selectors rather than the AX tree. |
| **`get`** | `(ref_id) → Option<&RefEntry>` | Looks up an entry by its ref identifier string. |
| **`entries_sorted`** | `() → Vec<(String, RefEntry)>` | Returns all entries sorted by the numeric portion of the ref ID (e.g. `e1`, `e2`, `e3`…). Ensures deterministic ordering for rendering and de-duplication. |
| **`remove`** | `(ref_id)` | Removes a single entry from the map. |
| **`clear`** | `()` | Clears all entries and resets `next_ref` to `1`. |
| **`next_ref_num`** | `() → usize` | Returns the current `next_ref` counter (the next ref ID that would be assigned). |
| **`set_next_ref_num`** | `(n)` | Sets the `next_ref` counter. Used after a snapshot to persist the counter across multiple snapshots so ref IDs are globally unique within a session. |

### 1.3 parse_ref

```rust
pub fn parse_ref(input: &str) -> Option<String>
```

Accepts three ref formats and normalizes them to the bare `eN` form:

| Input format | Example | Normalized output |
|--------------|---------|-------------------|
| `@eN` prefix | `@e5` | `"e5"` |
| `ref=eN` prefix | `ref=e5` | `"e5"` |
| Bare `eN` | `e5` | `"e5"` |

Any input that does not match the pattern `e` followed by only ASCII digits is rejected
(returning `None`). This prevents misinterpreting CSS selectors or XPath expressions as
ref identifiers.

### 1.4 resolve_element_center

```rust
pub async fn resolve_element_center(
    client: &CdpClient,
    session_id: &str,
    ref_map: &RefMap,
    selector_or_ref: &str,
    iframe_sessions: &HashMap<String, String>,
) -> Result<(f64, f64, String), String>
```

Resolves a ref or CSS selector to a pixel coordinate (x, y) and the effective CDP session
ID. The coordinate is suitable for mouse click targeting.

**Resolution strategy:**

1. **Ref path** — If `parse_ref` succeeds:
   - Look up the `RefEntry` in `ref_map`.
   - Resolve the effective session via `resolve_frame_session` (iframe routing).
   - **Fast path**: If `backend_node_id` is present, call `DOM.getBoxModel` with it and
     compute the center via `box_model_center`.
   - **Stale-node fallback**: If `DOM.getBoxModel` fails (node was removed or replaced),
     call `find_node_id_by_role_name` to re-query the AX tree for a fresh
     `backendDOMNodeId`, then retry `DOM.getBoxModel`.
2. **Selector path** — If `parse_ref` returns `None`, treat the input as a CSS/XPath
   selector and call `resolve_by_selector`, which evaluates a JavaScript expression using
   `getBoundingClientRect`.

Returns `(x, y, effective_session_id)` on success.

### 1.5 resolve_element_object_id

```rust
pub async fn resolve_element_object_id(
    client: &CdpClient,
    session_id: &str,
    ref_map: &RefMap,
    selector_or_ref: &str,
    iframe_sessions: &HashMap<String, String>,
) -> Result<(String, String), String>
```

Resolves a ref or selector to a CDP `objectId` and effective session ID. The `objectId`
is required by `Runtime.callFunctionOn` — the CDP method used by all property-query
functions to execute JavaScript on a specific DOM element.

**Resolution strategy mirrors `resolve_element_center`:**

1. **Ref path**: Look up entry, resolve session, try `DOM.resolveNode` with cached
   `backend_node_id`. On stale-node failure, re-query via `find_node_id_by_role_name`
   and retry `DOM.resolveNode`.
2. **Selector path**: Use `build_find_element_js` to evaluate `document.querySelector`
   (or XPath equivalent) and extract the `objectId` from the `Runtime.evaluate` result.

Returns `(object_id, effective_session_id)` on success.

### 1.6 resolve_ax_session / resolve_frame_session

#### resolve_ax_session

```rust
pub(super) fn resolve_ax_session<'a>(
    frame_id: Option<&str>,
    session_id: &'a str,
    iframe_sessions: &'a HashMap<String, String>,
) -> (serde_json::Value, &'a str)
```

Determines which CDP session and parameter set to use for an `Accessibility.getFullAXTree`
query. The routing logic:

- **Cross-origin iframe** (has a dedicated session in `iframe_sessions`): Return empty
  JSON params `{}` and the dedicated iframe session. Cross-origin sessions do not need
  a `frameId` parameter because they target the iframe document directly.
- **Same-origin iframe** (no dedicated session): Return `{"frameId": frame_id}` and the
  parent session. The `frameId` parameter tells CDP to scope the AX query to the iframe
  within the parent document.
- **Main frame** (`frame_id = None`): Return `{}` and the parent session.

#### resolve_frame_session

```rust
fn resolve_frame_session<'a>(
    frame_id: Option<&str>,
    session_id: &'a str,
    iframe_sessions: &'a HashMap<String, String>,
) -> &'a str
```

A simpler version used for element-level CDP calls (box model, resolve node, property
queries). Returns the dedicated iframe session if one exists, otherwise the parent
session. No `frameId` parameter is needed because these calls target a specific node
by `backend_node_id` or `objectId`.

### 1.7 find_node_id_by_role_name

```rust
async fn find_node_id_by_role_name(
    client: &CdpClient,
    session_id: &str,
    role: &str,
    name: &str,
    nth: Option<usize>,
    frame_id: Option<&str>,
    iframe_sessions: &HashMap<String, String>,
) -> Result<i64, String>
```

Re-queries `Accessibility.getFullAXTree` to find a node matching the given `role`,
`name`, and optional `nth` disambiguation index. This is the stale-node fallback
mechanism: when a cached `backend_node_id` no longer resolves (e.g. the page was
updated), the system can still locate the element by its semantic identity.

The function iterates over all non-ignored AX nodes, matches on `role` and `name`, and
uses `nth` to select the correct occurrence when duplicates exist. It uses the same
AX data source that built the ref map during snapshot, ensuring role/name matching is
consistent.

### 1.8 resolve_by_selector

```rust
async fn resolve_by_selector(
    client: &CdpClient,
    session_id: &str,
    selector: &str,
) -> Result<(f64, f64), String>
```

Resolves a CSS or XPath selector to a center coordinate using client-side JavaScript.
Uses `build_selector_js` to generate a self-contained JS expression:

- **CSS selectors**: `document.querySelector(selector).getBoundingClientRect()` → center.
- **XPath selectors** (prefixed with `xpath=`): `document.evaluate(…).singleNodeValue.getBoundingClientRect()` → center.

The JS expression is evaluated via `Runtime.evaluate` with `returnByValue: true`. Returns
`(x, y)` on success; errors if the selector matches no element.

### 1.9 box_model_center

```rust
fn box_model_center(model: &BoxModel) -> (f64, f64)
```

Computes the geometric center of an element from its CDP content quad. The content quad
is a flat array of 8 floats `[x1, y1, x2, y2, x3, y3, x4, y4]` representing the four
corners of the element's content box. The center is the average of all four x and y
coordinates:

```
x_center = (x1 + x2 + x3 + x4) / 4
y_center = (y1 + y2 + y3 + y4) / 4
```

Returns `(0.0, 0.0)` if the content quad has fewer than 8 values (malformed response).

### 1.10 Property Query Functions

All property queries follow the same pattern: resolve the element to an `objectId` via
`resolve_element_object_id`, then call `Runtime.callFunctionOn` with a JavaScript
function body that reads the desired property from `this` (the DOM element).

| Function | JS logic | Return type | Notes |
|----------|----------|-------------|-------|
| **`get_element_text`** | `this.innerText || this.textContent || ''` | `String` | Falls back to `textContent` when `innerText` is empty (e.g. hidden elements). |
| **`get_element_attribute`** | `this.getAttribute(attributeName)` | `Value` | Returns `Null` for missing attributes; returns the attribute value as a JSON primitive. |
| **`is_element_visible`** | Checks `rect.width > 0 && rect.height > 0 && visibility !== 'hidden' && display !== 'none' && opacity > 0` | `bool` | Comprehensive visibility check covering layout, CSS visibility, display, and opacity. |
| **`is_element_enabled`** | `!this.disabled` | `bool` | Returns `true` by default on failure (elements without a `disabled` property are assumed enabled). |
| **`is_element_checked`** | Multi-step logic mirroring Playwright's `getChecked()` | `bool` | 1. Native checkbox/radio input → `.checked`. 2. ARIA checked roles → `aria-checked === 'true'`. 3. Label association (`label.control`) → `.checked`. 4. Nested input fallback → `.checked`. |
| **`get_element_inner_text`** | `this.innerText || ''` | `String` | Returns only `innerText` (no `textContent` fallback). |
| **`get_element_inner_html`** | `this.innerHTML || ''` | `String` | Returns the raw HTML content of the element. |
| **`get_element_input_value`** | `typeof this.value === 'string' ? this.value : ''` | `String` | Only returns `.value` for elements that have a string `value` property (inputs, textareas, selects). |
| **`set_element_value`** | `this.value = newValue; dispatch input + change events` | `()` | Sets `.value` and fires both `input` and `change` events with `bubbles: true` to trigger React/Vue/Angular change detection. |
| **`get_element_bounding_box`** | `{ x, y, width, height }` from `getBoundingClientRect()` | `Value` | JSON object with viewport-relative coordinates. |
| **`get_element_count`** | `querySelectorAll(selector).length` or XPath `snapshotLength` | `i64` | Takes a selector directly (not a ref). Uses `build_count_elements_js` for CSS/XPath. |
| **`get_element_styles`** | `getComputedStyle(this)` → all or specific properties | `Value` | When `properties` is `Some`, returns only those named CSS properties; when `None`, returns all computed styles as a key-value map. |

---

## 2. snapshot.rs — Accessibility Tree Snapshot

The snapshot module converts the browser's accessibility tree into a compact, ref-annotated
text representation that the LLM can read and act on. It handles iframe recursion,
cursor-interactive element discovery, hidden-input promotion, and tree compaction.

### 2.1 Role Classification Constants

Three constant slices categorize ARIA roles by their function in the snapshot:

#### INTERACTIVE_ROLES

```rust
const INTERACTIVE_ROLES: &[&str] = &[
    "button", "link", "textbox", "checkbox", "radio", "combobox",
    "listbox", "menuitem", "menuitemcheckbox", "menuitemradio",
    "option", "searchbox", "slider", "spinbutton", "switch",
    "tab", "treeitem", "Iframe",
];
```

Elements with these roles **always receive a ref** — they are the primary interaction
targets for the agent (clicking, typing, selecting).

#### CONTENT_ROLES

```rust
const CONTENT_ROLES: &[&str] = &[
    "heading", "cell", "gridcell", "columnheader", "rowheader",
    "listitem", "article", "region", "main", "navigation",
];
```

Content-role elements receive a ref **only when they have a non-empty accessible name**.
Unnamed content nodes (e.g. an empty heading) are not ref-worthy.

#### STRUCTURAL_ROLES

```rust
const STRUCTURAL_ROLES: &[&str] = &[
    "generic", "group", "list", "table", "row", "rowgroup", "grid",
    "treegrid", "menu", "menubar", "toolbar", "tablist", "tree",
    "directory", "document", "application", "presentation", "none",
    "WebArea", "RootWebArea",
];
```

Structural roles organize the tree but are not interaction targets. They are **removed
during compaction** (`compact_tree`) to reduce noise in the LLM output.

#### INVISIBLE_CHARS

```rust
const INVISIBLE_CHARS: &[char] = &[
    '\u{FEFF}', '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{00A0}',
];
```

Unicode characters that are stripped from node names during rendering to prevent
zero-width or non-breaking space artifacts from polluting the snapshot text.

### 2.2 SnapshotOptions

```rust
#[derive(Default)]
pub struct SnapshotOptions {
    pub selector: Option<String>,
    pub interactive: bool,
    pub compact: bool,
    pub depth: Option<usize>,
    pub urls: bool,
}
```

| Field | Default | Effect |
|-------|---------|--------|
| `selector` | `None` | When set, only the DOM subtree rooted at the matching element is included in the snapshot. |
| `interactive` | `false` | When `true`, only ref-bearing (interactive) nodes are rendered; non-interactive nodes are skipped but their children are still traversed. |
| `compact` | `false` | When `true`, structural nodes without refs are removed after rendering, producing a denser tree. |
| `depth` | `None` | Maximum indentation depth to render. Nodes beyond this depth are omitted. |
| `urls` | `false` | When `true`, link elements are annotated with their resolved `href` URLs (requires additional CDP calls). |

### 2.3 TreeNode

```rust
struct TreeNode {
    role: String,
    name: String,
    level: Option<i64>,
    checked: Option<String>,
    expanded: Option<bool>,
    selected: Option<bool>,
    disabled: Option<bool>,
    required: Option<bool>,
    value_text: Option<String>,
    backend_node_id: Option<i64>,
    children: Vec<usize>,
    parent_idx: Option<usize>,
    has_ref: bool,
    ref_id: Option<String>,
    depth: usize,
    cursor_info: Option<CursorElementInfo>,
    url: Option<String>,
}
```

The internal representation of an accessibility node within the snapshot tree. Key
fields:

| Field | Purpose |
|-------|---------|
| `role` / `name` | ARIA role and accessible name — the primary identity of the node. |
| `level` | Heading level (1–6) for `heading` roles. |
| `checked` | Checked state as a string: `"true"`, `"false"`, or `"mixed"` (tristate). |
| `expanded` | Whether a collapsible container is open. |
| `selected` | Whether an option/tab is currently selected. |
| `disabled` / `required` | Form element states. |
| `value_text` | The current value of an input/combobox (distinct from `name`). |
| `backend_node_id` | CDP `backendDOMNodeId` for ref resolution. |
| `children` / `parent_idx` | Index-based tree structure using `Vec<usize>` references into the flat `tree_nodes` array. |
| `has_ref` / `ref_id` | Whether this node was assigned a ref, and the ref identifier (e.g. `"e3"`). |
| `depth` | Nesting depth (0 for roots), set by `set_depth` during tree construction. |
| `cursor_info` | Populated from `find_cursor_interactive_elements` for non-ARIA-interactive elements that are still clickable/focusable. |
| `url` | Resolved `href` for link elements (populated only when `options.urls` is `true`). |

#### TreeNode::empty / TreeNode::clear

- **`empty()`** creates a blank node with all fields at default values — used for
  ignored AX nodes that need a slot in the array but are never rendered.
- **`clear()`** resets all fields to defaults and clears `children` — used to blank
  out aggregated `StaticText` nodes that were merged into a sibling.

### 2.4 CursorElementInfo

```rust
#[derive(Clone)]
struct CursorElementInfo {
    kind: String,              // "clickable", "focusable", "editable"
    hints: Vec<String>,        // ["cursor:pointer", "onclick", "tabindex", "contenteditable"]
    text: String,              // textContent from the DOM element
    hidden_input_kind: Option<HiddenInputKind>,
    hidden_input_checked: Option<String>, // "true", "false", or "mixed"
}
```

Captures metadata about DOM elements that are not in `INTERACTIVE_ROLES` but are still
user-interactive (via `cursor:pointer`, `onclick`, `tabindex`, or `contenteditable`).
This information bridges the gap between the accessibility tree (which may not mark
these elements as interactive) and the real user-facing interactability of the page.

The `kind` field determines the annotation in the rendered snapshot:

- `"clickable"` — elements with `cursor:pointer` or `onclick`.
- `"editable"` — elements with `contenteditable`.
- `"focusable"` — elements with `tabindex` (but no cursor/onclick).

### 2.5 HiddenInputKind

```rust
enum HiddenInputKind {
    Radio,
    Checkbox,
}
```

Represents the type of a hidden `<input>` element found inside a cursor-interactive
container (typically a `<label>` wrapping a display:none radio/checkbox). These hidden
inputs are "promoted" — their role and checked state are merged into the parent element
so the LLM sees them as proper `radio` or `checkbox` nodes rather than generic clickable
containers.

| Method | Description |
|--------|-------------|
| `parse(s)` | Parses `"radio"` → `Radio`, `"checkbox"` → `Checkbox`, else `None`. |
| `as_role()` | Returns `"radio"` for `Radio`, `"checkbox"` for `Checkbox` — used to override the parent node's role during promotion. |

### 2.6 RoleNameTracker

```rust
struct RoleNameTracker {
    counts: HashMap<String, usize>,      // "role:name" → occurrence count
    entries: Vec<(usize, String)>,        // (node_index, key)
}
```

Tracks duplicate `role`+`name` combinations during ref assignment. When multiple nodes
share the same role and name (e.g. two `"button"` elements both named `"OK"`), the
tracker records their `nth` index (0, 1, 2…) so each can be uniquely identified during
stale-node fallback.

| Method | Description |
|--------|-------------|
| `new()` | Creates an empty tracker. |
| `track(role, name, node_idx)` | Records a node; returns the `nth` index for this role+name combination. Increments the occurrence counter. |
| `get_duplicates()` | Returns a `HashMap` of keys that appear more than once, with their total count. Nodes whose key is in this map get a non-`None` `nth` in their `RefEntry`. |

### 2.7 take_snapshot

```rust
pub async fn take_snapshot(
    client: &CdpClient,
    session_id: &str,
    options: &SnapshotOptions,
    ref_map: &mut RefMap,
    frame_id: Option<&str>,
    iframe_sessions: &HashMap<String, String>,
) -> Result<String, String>
```

The primary entry point. Produces a text representation of the page's accessibility tree,
annotated with refs for interactive elements. The full pipeline:

1. **Enable CDP domains** — `DOM.enable` and `Accessibility.enable` on the target session.

2. **Resolve selector scope** — If `options.selector` is set, use `Runtime.evaluate` to
   find the matching element, then `DOM.describeNode` with `depth: -1` to collect all
   `backendNodeIds` in its subtree via `collect_backend_node_ids`. This set determines
   which AX nodes become the effective roots.

3. **Enable domains on iframe session** — If the effective session differs from the
   parent (cross-origin iframe), defensively enable DOM and Accessibility on that session.

4. **Query AX tree** — `Accessibility.getFullAXTree` with the resolved session and params.
   The `resolve_ax_session` function determines the correct session and `frameId` parameter.

5. **Build tree** — `build_tree` converts the flat `AXNode` array into an indexed
   `TreeNode` array with parent-child links, StaticText aggregation, depth assignment,
   and root identification.

6. **Filter to selector subtree** — When a selector is active, identify "top-level" AX
   nodes: those whose `backend_node_id` is in the selector subtree but whose parent is
   not. These become the effective roots.

7. **Discover cursor-interactive elements** — `find_cursor_interactive_elements` runs a
   comprehensive JavaScript evaluation to find non-ARIA-interactive elements that are
   still clickable/focusable/editable.

8. **Promote hidden inputs** — `promote_hidden_inputs` merges hidden radio/checkbox
   inputs into their parent label/generic containers, converting them into proper
   `radio`/`checkbox` nodes with checked state.

9. **Assign refs** — Iterate over all tree nodes. Nodes in `INTERACTIVE_ROLES` always
   get a ref; nodes in `CONTENT_ROLES` get a ref only if they have a name; nodes that
   appear in `cursor_elements` also get a ref. Track role+name combinations via
   `RoleNameTracker` and assign `nth` indices to duplicates.

10. **Populate cursor_info** — For each ref-bearing node with a `backend_node_id`
    matching a cursor-interactive element, attach the `CursorElementInfo`.

11. **Resolve URLs** — If `options.urls` is `true`, resolve `href` attributes for all
    link elements with refs. Uses parallel `DOM.resolveNode` → `Runtime.callFunctionOn`
    calls (two phases: resolve objectIds, then fetch hrefs).

12. **Render tree** — `render_tree` produces the indented text output with role, name,
    state annotations, ref markers, cursor-info annotations, and value text.

13. **Recurse into iframes** — When `frame_id` is `None` (main frame only), find all
    `Iframe` nodes with refs and `backend_node_id`, resolve their child frame IDs via
    `resolve_iframe_frame_id`, and recursively call `take_snapshot` for each. The child
    snapshot text is inserted after the parent Iframe line, indented one level deeper.

14. **Compact** — If `options.compact` is `true`, apply `compact_tree` to remove
    structural nodes and collapse radio/checkbox.

15. **Final output** — Trim whitespace. If the result is empty, return
    `"no interactive elements"` (interactive mode) or `"empty page"` (full mode).

### 2.8 resolve_iframe_frame_id

```rust
async fn resolve_iframe_frame_id(
    client: &CdpClient,
    session_id: &str,
    backend_node_id: i64,
) -> Result<String, String>
```

Resolves the CDP frame ID of an iframe element by calling `DOM.describeNode` with
`depth: 1` (to include `contentDocument`). Two extraction paths:

1. **Primary**: `node.contentDocument.frameId` — standard for iframes with a content
   document.
2. **Fallback**: `node.frameId` — some iframe nodes carry the frame ID directly.

Returns the frame ID string on success; errors if neither path yields a value.

### 2.9 find_cursor_interactive_elements

```rust
async fn find_cursor_interactive_elements(
    client: &CdpClient,
    session_id: &str,
) -> Result<HashMap<i64, CursorElementInfo>, String>
```

Runs a single comprehensive JavaScript evaluation over the entire page body to discover
DOM elements that are user-interactive but not represented in `INTERACTIVE_ROLES`. The
JS script:

- Walks all elements via `querySelectorAll('*')`.
- Checks `getComputedStyle(el).cursor === 'pointer'`, `onclick`, `tabindex`, and
  `contenteditable`.
- Skips native interactive tags (`<a>`, `<button>`, `<input>`, `<select>`,
  `<textarea>`, `<details>`, `<summary>`).
- Skips elements with interactive ARIA roles.
- Skips elements that only inherit `cursor:pointer` from a parent.
- Skips zero-size and hidden elements.
- Detects hidden `<input type="radio|checkbox">` inside each candidate (common pattern:
  `<label>` wrapping a display:none input styled as a card).
- Tags each matched element with `data-__ab-ci` for batch backendNodeId resolution.

After the JS evaluation, the function batch-resolves `backendNodeIds` using:

1. `DOM.getDocument` → root `nodeId`.
2. `DOM.querySelectorAll("[data-__ab-ci]")` → all tagged elements' nodeIds.
3. Parallel `DOM.describeNode` calls → `backendNodeId` per tagged element.
4. Cleanup: removes all `data-__ab-ci` attributes via a second `Runtime.evaluate`.

The result is a `HashMap<i64, CursorElementInfo>` keyed by `backendNodeId`, containing
the kind, hints, text, and optional hidden-input metadata for each cursor-interactive
element.

### 2.10 promote_hidden_inputs

```rust
fn promote_hidden_inputs(
    tree_nodes: &mut [TreeNode],
    cursor_elements: &HashMap<i64, CursorElementInfo>,
)
```

Merges hidden radio/checkbox inputs into their parent `LabelText` or `generic` containers.
For each node whose `backend_node_id` maps to a `CursorElementInfo` with a
`hidden_input_kind`:

- Override the node's `role` to `"radio"` or `"checkbox"` (via `HiddenInputKind::as_role`).
- If the node's ARIA name is empty, fill it with the `CursorElementInfo.text` (DOM
  textContent).
- Copy the `hidden_input_checked` value into the node's `checked` field.

This promotion ensures the LLM sees properly typed radio/checkbox nodes instead of
generic clickable containers, enabling correct interaction decisions.

### 2.11 build_tree

```rust
fn build_tree(nodes: &[AXNode]) -> (Vec<TreeNode>, Vec<usize>)
```

Converts the flat CDP AX node list into an indexed tree structure. Returns
`(tree_nodes, root_indices)` where `root_indices` identifies nodes with no parent
(typically `RootWebArea`).

**Steps:**

1. **Create TreeNode array** — For each `AXNode`, extract role, name, value, and
   properties via `extract_ax_string` / `extract_properties`. Ignored nodes (except
   `RootWebArea`) and `InlineTextBox` nodes get `TreeNode::empty()` slots.

2. **Build parent-child links** — Iterate over `child_ids` in each AX node. Map CDP
   `nodeId` strings to array indices via `id_to_idx`. Populate `children` and `parent_idx`.

3. **Aggregate StaticText** — Continuous sequences of `StaticText` children are merged
   into the first node (names concatenated). Remaining nodes in the sequence are cleared.
   Single `StaticText` children whose name duplicates their parent's name are also cleared
   (de-duplication).

4. **Identify roots** — Nodes that are not a child of any other node are root candidates.

5. **Set depths** — `set_depth` recursively assigns `depth` starting from 0 at roots,
   incrementing by 1 per level.

### 2.12 render_tree

```rust
fn render_tree(
    nodes: &[TreeNode],
    idx: usize,
    indent: usize,
    output: &mut String,
    options: &SnapshotOptions,
)
```

Recursively renders the tree into indented text lines. Each line follows the format:

```
{indent}- {role} {name} [{attrs}] {cursor_kind} [{cursor_hints}]: {value}
```

**Rendering rules:**

- **Skipped nodes**: Empty-role nodes, `generic` nodes without refs with ≤1 child, and
  `StaticText` nodes with only invisible characters — their children are still rendered.
- **Root wrappers**: `RootWebArea` and `WebArea` are skipped; their children rendered
  directly.
- **Interactive mode**: When `options.interactive` is `true`, nodes without refs are
  skipped but their children are still traversed.
- **Depth limit**: Nodes beyond `options.depth` are omitted entirely.
- **Name display**: ARIA name is preferred. In interactive mode, `cursor_info.text` is
  used as a fallback when the ARIA name is empty.
- **Attributes**: `level`, `checked`, `expanded`, `selected`, `disabled`, `required`,
  `ref`, `url` — rendered in a bracket-enclosed list.
- **Cursor annotations**: `cursor_info.kind` and `cursor_info.hints` are appended after
  the attribute list.
- **Value text**: Appended after a `:` separator when present and distinct from the name.

### 2.13 compact_tree

```rust
fn compact_tree(tree: &str, interactive: bool) -> String
```

Post-processes the rendered tree text to remove structural noise. Operates line-by-line:

1. **Mark keep-lines**: Lines containing `ref=` or `: ` (value annotations) are kept.
2. **Mark ancestors**: For each kept line, mark all ancestor lines (those with less
   indentation) as kept, ensuring the tree structure remains connected.
3. **Filter**: Remove all unmarked lines.
4. **Fallback**: If the result is empty and `interactive` is `true`, return
    `"no interactive elements"`.

This produces a denser tree where only ref-bearing nodes and their structural context
remain, reducing LLM prompt size while preserving navigability.

### 2.14 Supporting Functions

#### extract_ax_string / extract_ax_string_opt

```rust
fn extract_ax_string(value: &Option<AXValue>) -> String
fn extract_ax_string_opt(value: &Option<AXValue>) -> Option<String>
```

Extract the string representation from a CDP `AXValue` object. Handles `String`,
`Number`, and `Bool` value types. `extract_ax_string_opt` returns `None` for empty
strings (used for `value_text` to avoid storing empty values).

#### extract_properties

```rust
fn extract_properties(props: &Option<Vec<AXProperty>>) -> NodeProperties
```

Extracts the six recognized ARIA state properties from an AX node's property list:
`level`, `checked`, `expanded`, `selected`, `disabled`, `required`. Returns them as a
tuple of `Option` values.

#### build_dedup_set

```rust
fn build_dedup_set(ref_map: &RefMap) -> HashSet<String>
```

Builds a case-insensitive set of all ref-bearing node names from the `RefMap`. Used to
de-duplicate cursor-interactive elements whose textContent matches an already-ref'd ARIA
node, preventing redundant ref assignment.

#### collect_backend_node_ids

```rust
fn collect_backend_node_ids(node: &Value, ids: &mut HashSet<i64>)
```

Recursively walks a CDP `DOM.describeNode` result (including `children`, `shadowRoots`,
and `contentDocument`) to collect all `backendNodeId` values. Used to determine the set
of AX nodes within a selector-scoped subtree.

#### count_indent

```rust
fn count_indent(line: &str) -> usize
```

Calculates the indentation level of a rendered line by counting leading spaces and
dividing by 2 (each indent level = 2 spaces).