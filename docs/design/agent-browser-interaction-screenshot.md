# Agent Browser Interaction & Screenshot Modules

> Design documentation for the two core browser automation modules:
> **`interaction.rs`** — CDP-driven interaction primitives, and **`screenshot.rs`** —
> screenshot capture with numbered accessibility annotations.

---

## 1. `interaction.rs` — Browser Interaction Primitives

This module provides low-level CDP (Chrome DevTools Protocol)–driven functions
that simulate human-like browser interactions. Every function receives a
`CdpClient`, `session_id`, `RefMap`, and an optional `iframe_sessions` map so
that interactions work correctly across iframe boundaries.

All element targeting uses either a CSS selector or an accessibility-ref string
(`eN`). The `resolve_element_center` and `resolve_element_object_id` helpers
(from the sibling `element` module) translate selectors/refs into CDP
coordinates or remote object IDs, routing to the correct iframe session when
needed.

### 1.1 `click`

```rust
pub async fn click(
    client, session_id, ref_map, selector_or_ref,
    button, click_count, iframe_sessions
) -> Result<(), String>
```

Resolves the target element's center coordinates via `resolve_element_center`,
then delegates to `dispatch_click`. The `button` parameter accepts `"left"`,
`"right"`, or `"middle"`; `click_count` defaults to `1` for a single click.

### 1.2 `dblclick`

```rust
pub async fn dblclick(
    client, session_id, ref_map, selector_or_ref, iframe_sessions
) -> Result<(), String>
```

Simply calls `click` with `button = "left"` and `clickCount = 2`. CDP's
`mousePressed` + `mouseReleased` pair with `clickCount=2` produces the
standard `dblclick` DOM event.

### 1.3 `hover`

```rust
pub async fn hover(
    client, session_id, ref_map, selector_or_ref, iframe_sessions
) -> Result<(), String>
```

Resolves the element center and dispatches a single
`Input.dispatchMouseEvent` with `type = "mouseMoved"` — no button pressed, no
click count. This moves the virtual cursor to the element's center, triggering
hover-related CSS and JS effects.

### 1.4 `fill`

```rust
pub async fn fill(
    client, session_id, ref_map, selector_or_ref,
    value, iframe_sessions
) -> Result<(), String>
```

Three-step sequence that replaces the entire content of an input field:

1. **Focus** — `Runtime.callFunctionOn` calling `this.focus()`.
2. **Clear** — `Runtime.callFunctionOn` executing JS that calls
   `this.select()` (for `<input>`/`<textarea>` with a `.select` method) and
   then sets `this.value = ''` while dispatching an `input` event with
   `bubbles: true`.
3. **Insert** — `Input.insertText` with the full replacement string, sent on
   the **parent** `session_id` (not the iframe session) so that Chrome treats
   it as top-level keyboard input.

This mirrors Playwright's `fill()` behaviour: a single atomic text replacement
rather than character-by-character typing.

### 1.5 `type_text`

```rust
pub async fn type_text(
    client, session_id, ref_map, selector_or_ref,
    text, clear, delay_ms, iframe_sessions
) -> Result<(), String>
```

Two-step approach:

1. **Focus the element** — `Runtime.callFunctionOn` with `this.focus()`.
2. **Optionally clear** — when `clear = true`, selects existing content and
   sets `value = ''` with an `input` event (same JS as `fill`'s clear step).
3. **Delegate to `type_text_into_active_context`** — character-by-character
   input with optional inter-key delay.

### 1.6 `type_text_into_active_context`

```rust
pub async fn type_text_into_active_context(
    client, session_id, text, delay_ms
) -> Result<(), String>
```

Iterates over each character in `text` and dispatches the appropriate CDP
event:

- **Control characters** (`\n`, `\r`, `\t`) → dispatched via
  `Input.dispatchKeyEvent` with `keyDown` + `keyUp`, using the key info
  returned by `char_to_key_info` and `key_text`. For example, Enter gets
  `key = "Enter"`, `code = "Enter"`, `windowsVirtualKeyCode = 13`, and
  `text = "\r"`.

- **Printable characters** → dispatched via `Input.insertText` with the
  character as a single-character string. This avoids the problem where
  VS Code/Electron webviews reject repeated `dispatchKeyEvent` calls carrying
  printable `text`.

- **Inter-key delay** — after each character, if `delay_ms > 0`, the function
  sleeps for the specified duration via `tokio::time::sleep`.

This split (control keys via key events, printable via insertText) is critical
for compatibility with webviews that intercept key events differently from
text insertion events.

### 1.7 `press_key` / `press_key_with_modifiers`

```rust
pub async fn press_key(client, session_id, key) -> Result<(), String>
pub async fn press_key_with_modifiers(
    client, session_id, key, modifiers
) -> Result<(), String>
```

Dispatches a `keyDown` + `keyUp` pair using `named_key_info` to resolve the
key string into `(key, code, windowsVirtualKeyCode)`.

**Named keys** are resolved via `named_key_info` which maps aliases like
`"enter"` → `"Enter"`, `"up"` → `"ArrowUp"`, `"esc"` → `"Escape"`, and so on.
Single-character strings fall through to `char_to_key_info`.

**Modifier bitmask** follows CDP convention:
| Value | Modifier |
|-------|----------|
| 1     | Alt      |
| 2     | Control  |
| 4     | Meta (Cmd) |
| 8     | Shift    |

**Text suppression** — when Control (bit 2) or Meta (bit 4) is active,
`text` is set to `None` on the `keyDown` event. This prevents Chrome from
inserting literal text during command chords (e.g. Ctrl+A should select all,
not type the letter "a").

### 1.8 `scroll`

```rust
pub async fn scroll(
    client, session_id, ref_map, selector_or_ref,
    delta_x, delta_y, iframe_sessions
) -> Result<(), String>
```

Two modes:

- **Element-relative** — when `selector_or_ref` is `Some`, resolves the
  element's remote object ID and calls `Runtime.callFunctionOn` with
  `function(dx, dy) { this.scrollBy(dx, dy); }`, passing `delta_x` and
  `delta_y` as call arguments.

- **Viewport-relative** — when `selector_or_ref` is `None`, evaluates
  `window.scrollBy(deltaX, deltaY)` via `Runtime.evaluate`.

### 1.9 `select_option`

```rust
pub async fn select_option(
    client, session_id, ref_map, selector_or_ref,
    values, iframe_sessions
) -> Result<(), String>
```

Resolves the `<select>` element and calls `Runtime.callFunctionOn` with JS
that:

1. Iterates over `this.options`.
2. Sets `opt.selected = true` when `opt.value` or `opt.textContent.trim()`
   matches one of the provided `values`.
3. Dispatches a `change` event with `bubbles: true`.

The `values` argument is passed as a JSON array call argument.

### 1.10 `check` / `uncheck`

```rust
pub async fn check(...)
pub async fn uncheck(...)
```

Both follow the same **verify-then-click-then-reverify** pattern:

1. **Check current state** — calls `is_element_checked` (from the `element`
   module).
2. **If state needs toggling** — dispatches a `click` with `button = "left"`,
   `clickCount = 1`.
3. **Verify the click worked** — re-checks `is_element_checked`. If the
   coordinate-based CDP click failed to toggle the state (common with hidden
   inputs, label-associated checkboxes, or overlay-obscured elements), falls
   back to `js_click_checkbox`.

`check` proceeds only if the element is currently unchecked; `uncheck` only if
it is currently checked.

### 1.11 `js_click_checkbox` (internal)

```rust
async fn js_click_checkbox(...)
```

Fallback for when the coordinate-based CDP click did not toggle the
checkbox/radio state. Uses `Runtime.callFunctionOn` with JS that follows
label-association resolution (mirroring Playwright):

1. **Native `<input>`** — if the element is an `<input type="checkbox">` or
   `<input type="radio">`, calls `.click()` directly.
2. **Label → control** — if the element is inside a `<label>` (or is a
   `<label>` itself), calls `label.control.click()`.
3. **Nested input** — if the element contains a descendant
   `<input type="checkbox|radio">`, clicks that input.
4. **ARIA role control** — otherwise, calls `.click()` on the element itself.

### 1.12 `focus`

```rust
pub async fn focus(...)
```

Resolves the element's remote object ID and calls `Runtime.callFunctionOn`
with `function() { this.focus(); }`.

### 1.13 `clear`

```rust
pub async fn clear(...)
```

Resolves the element's remote object ID and calls `Runtime.callFunctionOn`
with JS that:

1. Calls `this.focus()`.
2. Sets `this.value = ''`.
3. Dispatches `input` and `change` events with `bubbles: true`.

### 1.14 `select_all`

```rust
pub async fn select_all(...)
```

Resolves the element's remote object ID and calls `Runtime.callFunctionOn`
with JS that:

1. Calls `this.focus()`.
2. If `typeof this.select === 'function'` (native `<input>`/`<textarea>`) —
   calls `this.select()`.
3. Otherwise — creates a `Range`, calls `range.selectNodeContents(this)`, then
   `window.getSelection().removeAllRanges()` + `addRange(range)`.

This is the equivalent of Ctrl+A / Cmd+A selection for editable elements.

### 1.15 `scroll_into_view`

```rust
pub async fn scroll_into_view(...)
```

Resolves the element's remote object ID and calls `Runtime.callFunctionOn`
with `function() { this.scrollIntoView({ block: 'center', inline: 'center' }); }`.
Centers the element both vertically and horizontally within the viewport.

### 1.16 `dispatch_event`

```rust
pub async fn dispatch_event(
    client, session_id, ref_map, selector_or_ref,
    event_type, event_init, iframe_sessions
) -> Result<(), String>
```

Resolves the element and calls `Runtime.callFunctionOn` with JS:
`this.dispatchEvent(new Event(eventType, eventInit))`.

- `event_type` — the DOM event name (e.g. `"focus"`, `"blur"`,
  `"custom-event"`).
- `event_init` — optional `serde_json::Value` that is serialized into the
  `EventInit` dictionary. When omitted, defaults to `{ bubbles: true }`.

### 1.17 `highlight`

```rust
pub async fn highlight(...)
```

Resolves the element and calls `Runtime.callFunctionOn` with JS that:

1. Sets `this.style.outline = '2px solid red'` and `outlineOffset = '2px'`.
2. After a 3-second timeout, removes the outline and outlineOffset.

This provides a brief visual indicator for debugging or agent feedback.

### 1.18 `tap_touch`

```rust
pub async fn tap_touch(...)
```

Resolves the element center and dispatches two CDP touch events:

1. `Input.dispatchTouchEvent` with `type = "touchStart"` and a single
   `touchPoints` entry at `(x, y)`.
2. `Input.dispatchTouchEvent` with `type = "touchEnd"` and empty
   `touchPoints`.

Used for mobile/touch device simulation where `dispatchMouseEvent` would not
trigger touch-specific event handlers.

### 1.19 Internal Helpers

#### `dispatch_click`

```rust
async fn dispatch_click(client, session_id, x, y, button, click_count)
```

Three-step CDP mouse event sequence:

1. **Move** — `Input.dispatchMouseEvent` with `type = "mouseMoved"` to
   position the cursor at `(x, y)`.
2. **Press** — `Input.dispatchMouseEvent` with `type = "mousePressed"`,
   setting `button`, `buttons` (1=left, 2=right, 4=middle), and `clickCount`.
3. **Release** — `Input.dispatchMouseEvent` with `type = "mouseReleased"`,
   setting `buttons = 0` to indicate all buttons released.

#### `char_to_key_info`

```rust
fn char_to_key_info(ch: char) -> (String, String, i32)
```

Maps a character to the `(key, code, windowsVirtualKeyCode)` triple needed by
`Input.dispatchKeyEvent`. Classification:

- **Control chars** — `\n`/`\r` → Enter (VK 13), `\t` → Tab (VK 9), `' '` →
  Space (VK 32).
- **ASCII letters** — `key = ch`, `code = "Key{Upper}"`, `VK = uppercase
  ASCII value` (e.g. 'a' → `("a", "KeyA", 65)`).
- **ASCII digits** — `key = ch`, `code = "Digit{ch}"`, `VK = ASCII value`
  (e.g. '0' → `("0", "Digit0", 48)`).
- **Punctuation** — delegates to `punctuation_key_info`.
- **Unmapped** — returns `(ch.to_string(), "", 0)`, signalling that
  `type_text_into_active_context` should use `Input.insertText` instead.

#### `punctuation_key_info`

```rust
fn punctuation_key_info(ch: char) -> (&'static str, i32)
```

Returns the DOM `KeyboardEvent.code` and Windows virtual-key code for
punctuation characters on a US keyboard layout. Uses the VK_OEM_* constants
rather than raw ASCII values to avoid collisions — notably, `.` (ASCII 46)
must map to VK 190 (VK_OEM_PERIOD), not VK 46 (VK_DELETE).

| Character(s) | Code         | VK   |
|---------------|--------------|------|
| `;` `:`       | Semicolon    | 186  |
| `=` `+`       | Equal        | 187  |
| `,` `<`       | Comma        | 188  |
| `-` `_`       | Minus        | 189  |
| `.` `>`       | Period       | 190  |
| `/` `?`       | Slash        | 191  |
| `` ` `` `~`   | Backquote    | 192  |
| `[` `{`       | BracketLeft  | 219  |
| `\` `|`       | Backslash    | 220  |
| `]` `}`       | BracketRight | 221  |
| `'` `"`       | Quote        | 222  |

Unmapped characters return `("", 0)`.

#### `named_key_info`

```rust
fn named_key_info(key: &str) -> (String, String, i32)
```

Maps human-friendly key names to the `(key, code, VK)` triple. Accepts both
Playwright-style names (`"Enter"`, `"ArrowUp"`) and short aliases (`"return"`,
`"up"`, `"esc"`):

| Name(s)               | Key        | Code        | VK  |
|------------------------|------------|-------------|-----|
| enter / return         | Enter      | Enter       | 13  |
| tab                    | Tab        | Tab         | 9   |
| escape / esc           | Escape     | Escape      | 27  |
| backspace              | Backspace  | Backspace   | 8   |
| delete                 | Delete     | Delete      | 46  |
| arrowup / up           | ArrowUp    | ArrowUp     | 38  |
| arrowdown / down       | ArrowDown  | ArrowDown   | 40  |
| arrowleft / left       | ArrowLeft  | ArrowLeft   | 37  |
| arrowright / right     | ArrowRight | ArrowRight  | 39  |
| home                   | Home       | Home        | 36  |
| end                    | End        | End         | 35  |
| pageup                 | PageUp     | PageUp      | 33  |
| pagedown               | PageDown   | PageDown    | 34  |
| space / " "            | " "        | Space       | 32  |

Single-character strings fall through to `char_to_key_info`. Unknown
multi-character strings return `(key, key, 0)` as a passthrough.

#### `key_text`

```rust
fn key_text(key_name: &str) -> Option<String>
```

Returns the `text` value that CDP `Input.dispatchKeyEvent` needs on the
`keyDown` event so that Chrome performs the default action:

- `"Enter"` → `"\r"` (submits forms)
- `"Tab"` → `"\t"` (moves focus)
- `" "` → `" "` (space insertion)
- Single printable characters → themselves
- Non-printable named keys (ArrowUp, Escape, etc.) → `None`

#### Tests

The module includes unit tests that verify:

- **`test_char_to_key_info_matches_playwright_layout`** — every character in
  Playwright's USKeyboardLayout returns the correct `code` and `VK`, using
  values taken verbatim from Playwright's JS layout definition.
- **`test_period_is_not_vk_delete`** — regression test ensuring `.` never maps
  to VK 46 (VK_DELETE), which was the bug that prompted the VK_OEM_* mapping.
- **`test_unmapped_chars_return_zero_keycode`** — characters outside the US
  layout (`@`, `€`, `你`, etc.) return `("", 0)`.
- **`test_key_text_returns_correct_text_for_special_keys`** — Enter/Tab/Space
  produce the correct CDP text values; non-printable keys produce `None`.

---

## 2. `screenshot.rs` — Screenshot Capture with Annotations

This module captures browser screenshots via CDP and optionally overlays
numbered annotations derived from the accessibility tree. The annotation system
mirrors the Node.js screenshot `annotate` mode, allowing an LLM agent to
identify interactive elements by their numbered labels.

### 2.1 Data Structures

#### `Rect` (private)

```rust
struct Rect { x: f64, y: f64, width: f64, height: f64 }
```

Internal bounding rectangle in viewport coordinates. Used for raw annotation
rects, target selector rects, and overlap calculations.

#### `RawAnnotation` (private)

```rust
struct RawAnnotation {
    ref_id: String,    // e.g. "e42"
    number: u64,       // extracted numeric part of ref_id
    role: String,      // accessibility role (e.g. "button", "link")
    name: Option<String>, // accessible name (label text, alt text)
    rect: Rect,        // bounding rectangle in viewport space
}
```

Intermediate annotation gathered from the accessibility tree before projection
and filtering. The `number` is extracted by stripping the `"e"` prefix from
`ref_id` and parsing the remainder as a `u64`.

#### `AnnotationBox` (public)

```rust
pub struct AnnotationBox {
    pub x: i64, pub y: i64, pub width: i64, pub height: i64
}
```

Integer-coordinate bounding box in the final projected coordinate space.
Serialized as `"box"` in the output JSON. Values are rounded from `f64` via
`round()`.

#### `ScreenshotAnnotation` (public)

```rust
pub struct ScreenshotAnnotation {
    pub ref_id: String,
    pub number: u64,
    pub role: String,
    pub name: Option<String>,
    pub box_: AnnotationBox,
}
```

Final annotation after projection. Has a custom `Serialize` impl that outputs
fields as `ref`, `number`, `role`, `name` (only when present), and `box`.

#### `ScreenshotResult` (public)

```rust
pub struct ScreenshotResult {
    pub path: String,             // filesystem path where image was saved
    pub base64: String,           // raw base64-encoded image data from CDP
    pub annotations: Vec<ScreenshotAnnotation>,
}
```

Complete result of a screenshot capture. Returned by `take_screenshot`.

#### `ScreenshotOptions` (public)

```rust
pub struct ScreenshotOptions {
    pub selector: Option<String>,   // CSS selector or ref to capture/target
    pub path: Option<String>,       // explicit save path (overrides auto-naming)
    pub full_page: bool,            // capture entire document height
    pub format: String,             // "png" or "jpeg"
    pub quality: Option<i32>,       // JPEG quality (1–100, default 80 for jpeg)
    pub annotate: bool,             // overlay numbered annotations
    pub output_dir: Option<String>, // directory for auto-named screenshots
}
```

Default values: `selector = None`, `path = None`, `full_page = false`,
`format = "png"`, `quality = None`, `annotate = false`, `output_dir = None`.

### 2.2 `take_screenshot`

```rust
pub async fn take_screenshot(
    client, session_id, ref_map, options, iframe_sessions
) -> Result<ScreenshotResult, String>
```

Orchestrates the full screenshot pipeline:

1. **Resolve target rect** — if `annotate` is enabled and a `selector` is
   provided, calls `get_rect_for_selector` to get the target element's
   bounding rectangle. This rect is used both for clipping and for
   annotation filtering.

2. **Collect annotations** — if `annotate` is enabled, calls
   `collect_annotations` to gather all interactive/content elements from the
   accessibility tree.

3. **Filter annotations** — calls `filter_annotations` to keep only
   annotations whose rects overlap with the target rect (or all annotations
   if no target selector).

4. **Inject overlay** — if annotating and there are overlay items, calls
   `inject_annotation_overlay` to render numbered red boxes in the page DOM.

5. **Capture** — calls `capture_screenshot_base64` via CDP. The overlay is
   visible in the captured image.

6. **Remove overlay** — immediately removes the injected overlay DOM elements
   so the page is not permanently modified.

7. **Project annotations** — adjusts annotation coordinates relative to the
   target rect (selector mode) or adds scroll offsets (full-page mode) via
   `project_annotations`.

8. **Save** — decodes base64 and writes the image to disk via `save_screenshot`.

9. **Return** — `ScreenshotResult` with path, base64 data, and projected
   annotations.

The overlay injection → capture → removal sequence ensures annotations appear
in the screenshot but do not persist in the live page.

### 2.3 `capture_screenshot_base64`

```rust
async fn capture_screenshot_base64(
    client, session_id, ref_map, options, iframe_sessions
) -> Result<String, String>
```

Dispatches `Page.captureScreenshot` via CDP with parameters derived from
`ScreenshotOptions`:

- **Format/quality** — `format` passed directly; JPEG quality defaults to 80
  when format is `"jpeg"` and no explicit quality is set.
- **Full-page capture** — calls `Page.getLayoutMetrics` first to obtain
  `contentSize` (or `cssContentSize`), then sets `clip` to the full content
  dimensions with `scale = 1.0` and `captureBeyondViewport = true`.
- **Selector capture** — when not full-page and a selector is given, resolves
  the element's bounding rect and sets `clip` to that rect.
- **Viewport capture** — when neither full-page nor selector, captures the
  current viewport with no clip (CDP defaults to viewport).

Returns the `data` field from CDP's response (base64-encoded image).

### 2.4 `collect_annotations`

```rust
async fn collect_annotations(
    client, session_id, ref_map
) -> Result<Vec<RawAnnotation>, String>
```

Builds annotations from the accessibility ref map:

1. **Get sorted entries** — calls `ref_map.entries_sorted()` to get all
   accessibility refs in order.

2. **Filter entries with backend_node_ids** — only entries that have a
   `backend_node_id` can be resolved to DOM nodes.

3. **Batch-resolve node IDs** — dispatches concurrent
   `DOM.resolveNode` calls for all `backend_node_id` values, resolving them
   to `objectId` strings. Uses `futures_util::future::join_all` for
   parallelism.

4. **Batch-get bounding rects** — for each resolved `objectId`, calls
   `get_rect_for_object` (concurrently via `join_all`) to retrieve
   `getBoundingClientRect()` results.

5. **Build RawAnnotations** — for each element with a non-zero rect, extracts
   the numeric `number` from the `ref_id` (stripping the `"e"` prefix),
   populates `role` and `name` from the ref entry, and stores the rect.

Only elements with positive `width` and `height` are included (zero-size
elements are invisible and not useful for annotation).

### 2.5 `get_rect_for_selector`

```rust
async fn get_rect_for_selector(
    client, session_id, ref_map, selector, iframe_sessions
) -> Result<Option<Rect>, String>
```

Resolves a selector/ref to an object ID and delegates to
`get_rect_for_object`.

### 2.6 `get_rect_for_object`

```rust
async fn get_rect_for_object(
    client, session_id, object_id
) -> Result<Option<Rect>, String>
```

Calls `Runtime.callFunctionOn` with JS:
`function() { const rect = this.getBoundingClientRect(); return { x, y, width, height }; }`.

Parses the returned JSON value through `parse_rect`.

### 2.7 `filter_annotations`

```rust
fn filter_annotations(annotations, target_rect) -> Vec<RawAnnotation>
```

When a `target_rect` is provided (selector mode), keeps only annotations whose
rects overlap with the target rect via the `overlaps` function. When
`target_rect` is `None`, all annotations pass through. Results are sorted by
`number` for stable ordering.

### 2.8 `overlaps`

```rust
fn overlaps(left: &Rect, right: &Rect) -> bool
```

Standard axis-aligned bounding-box overlap test:
`left.x < right.x + right.width && left.x + left.width > right.x &&
 left.y < right.y + right.height && left.y + left.height > right.y`.

### 2.9 `inject_annotation_overlay`

```rust
async fn inject_annotation_overlay(
    client, session_id, annotations
) -> Result<(), String>
```

Injects a styled `<div>` overlay into the page DOM via `Runtime.evaluate`.
The overlay:

- Has `id = "__agent_browser_annotations__"` (constant
  `ANNOTATION_OVERLAY_ID`).
- Uses `position: absolute; top: 0; left: 0; pointer-events: none;
  z-index: 2147483647` so it covers everything but does not intercept clicks.
- For each annotation, creates a child `<div>` with:
  - A **red border box** — `border: 2px solid rgba(255,0,0,0.8)` positioned
    at the annotation's viewport coordinates (adjusted for scroll offset).
  - A **number label** — a small `<div>` with red background, white monospace
    text showing the annotation number, positioned above or inside the box
    depending on available space.

If a previous overlay exists (same ID), it is removed before injection.

### 2.10 `remove_annotation_overlay`

```rust
async fn remove_annotation_overlay(client, session_id) -> Result<(), String>
```

Evaluates JS that finds the overlay element by ID and removes it from the DOM.
Called immediately after screenshot capture to restore the page to its original
state.

### 2.11 `project_annotations`

```rust
fn project_annotations(
    annotations, target_rect, scroll
) -> Vec<ScreenshotAnnotation>
```

Adjusts annotation coordinates based on capture mode:

- **Selector mode** (`target_rect` present) — subtracts the target rect's
  origin from each annotation rect, making coordinates relative to the
  captured element. This matches the pixel space of the clipped screenshot.
- **Full-page mode** (`scroll` present) — adds `scrollX` and `scrollY` to
  annotation positions. Since `getBoundingClientRect()` returns viewport-relative
  coords but a full-page capture includes scrolled content, this shifts
  annotations into document space.
- **Viewport mode** (neither present) — coordinates are already in viewport
  space and no adjustment is needed.

All coordinates are rounded to `i64` via `round()` in the resulting
`AnnotationBox`.

### 2.12 `save_screenshot`

```rust
fn save_screenshot(base64_data, explicit_path, ext, output_dir) -> Result<String, String>
```

Determines the save path and writes the image:

- **Explicit path** — if `explicit_path` is provided, uses it directly.
- **Auto-named path** — otherwise, resolves the output directory:
  - `output_dir` if specified.
  - `~/.agent-browser/tmp/screenshots/` (via `dirs::home_dir()`) as default.
  - Falls back to the system temp dir if no home directory is available.
  - Creates the directory if it doesn't exist.
  - Names the file `screenshot-{timestamp_ms}.{ext}` where `ext` is `"jpg"`
    for JPEG or `"png"` for PNG.

Decodes base64 via `base64::Engine::decode` (standard engine) and writes bytes
via `std::fs::write`. Returns the full filesystem path on success.

### 2.13 `get_scroll_offsets`

```rust
async fn get_scroll_offsets(client, session_id) -> Result<(f64, f64), String>
```

Evaluates `({x: window.scrollX || 0, y: window.scrollY || 0})` via
`Runtime.evaluate` and returns the current scroll position. Used by
`take_screenshot` for full-page annotation projection.

### 2.14 Helper Functions

#### `parse_rect`

```rust
fn parse_rect(value: &Value) -> Option<Rect>
```

Extracts `x`, `y`, `width`, `height` from a JSON object. Returns `None` if any
field is missing or not a number.

#### `round`

```rust
fn round(value: f64) -> i64
```

Standard rounding: `value.round() as i64`.

#### `get_screenshot_dir`

```rust
fn get_screenshot_dir() -> PathBuf
```

Returns `~/.agent-browser/tmp/screenshots/` if `dirs::home_dir()` is
available, otherwise `{temp_dir}/agent-browser/screenshots/`.

### 2.15 Tests

The module includes unit tests for the pure projection and filtering logic:

- **`filters_annotations_to_target_overlap`** — verifies that
  `filter_annotations` keeps only annotations overlapping the target rect and
  discards those outside it.
- **`projects_selector_annotations_relative_to_target`** — verifies that
  selector-mode projection subtracts the target rect origin from annotation
  coordinates.
- **`projects_full_page_annotations_to_document_space`** — verifies that
  full-page-mode projection adds scroll offsets to annotation positions.

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                    Agent Runtime (LLM loop)                  │
│                                                             │
│  "Click the Login button"  ──►  tool call: click(e7)        │
│  "Take a screenshot"       ──►  tool call: screenshot()     │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              Browser Tool (agent-browser crate)              │
│                                                             │
│  ┌──────────────┐    ┌──────────────────────────────────┐   │
│  │ interaction  │    │          screenshot               │   │
│  │              │    │                                  │   │
│  │ click        │    │ take_screenshot                  │   │
│  │ dblclick     │    │   ├─ collect_annotations          │   │
│  │ hover        │    │   ├─ filter_annotations           │   │
│  │ fill         │    │   ├─ inject_annotation_overlay    │   │
│  │ type_text    │    │   ├─ capture_screenshot_base64    │   │
│  │ press_key    │    │   ├─ remove_annotation_overlay    │   │
│  │ scroll       │    │   ├─ project_annotations          │   │
│  │ select_option│    │   └─ save_screenshot              │   │
│  │ check/uncheck│    │                                  │   │
│  │ focus        │    │ annotation pipeline:              │   │
│  │ clear        │    │   AX tree → DOM.resolveNode →    │   │
│  │ select_all   │    │   getBoundingClientRect →        │   │
│  │ scroll_into  │    │   numbered overlay → capture →   │   │
│  │ dispatch_evt │    │   remove overlay → project       │   │
│  │ highlight    │    │                                  │   │
│  │ tap_touch    │    │                                  │   │
│  └──────────────┘    └──────────────────────────────────┘   │
│           │                       │                         │
│           ▼                       ▼                         │
┌─────────────────────────────────────────────────────────────┐
│                     CdpClient (CDP bridge)                   │
│                                                             │
│  Input.dispatchMouseEvent   Page.captureScreenshot          │
│  Input.dispatchKeyEvent     Page.getLayoutMetrics           │
│  Input.insertText           DOM.resolveNode                 │
│  Input.dispatchTouchEvent   Runtime.evaluate                │
│  Runtime.callFunctionOn     Runtime.callFunctionOn          │
└─────────────────────────────────────────────────────────────┘
```

---

## Key Design Decisions

### Why `Input.insertText` for printable characters?

VS Code and Electron webviews intercept `dispatchKeyEvent` differently from
text insertion events. Repeated `dispatchKeyEvent` calls with printable `text`
are rejected by these environments. By routing printable characters through
`Input.insertText` and reserving `dispatchKeyEvent` for control keys (Enter,
Tab), the module achieves broad compatibility without sacrificing correctness.

### Why VK_OEM_* codes for punctuation?

The Windows virtual-key codes for punctuation keys (VK_OEM_PERIOD = 190,
VK_OEM_COMMA = 188, etc.) differ from their ASCII values. Using ASCII codes
causes misidentification — most critically, `.` (ASCII 46) collides with
VK_DELETE (0x2E = 46). The `punctuation_key_info` function uses the correct
OEM codes, matching Playwright's USKeyboardLayout mapping verbatim.

### Why inject → capture → remove for annotations?

Injecting a DOM overlay before capture ensures numbered labels are visible in
the screenshot image, giving the LLM a visual reference for element
identification. Removing the overlay immediately after capture prevents the
page from being permanently modified — the overlay has `pointer-events: none`
so it doesn't interfere with interaction even during the brief injection
window.

### Why verify-and-retry for checkbox toggling?

Coordinate-based CDP clicks may fail to toggle checkbox state when the target
input is hidden (common with styled checkboxes), obscured by overlays, or
associated with a `<label>` element that proxies clicks. The verify → click →
reverify → JS-fallback pattern ensures reliable toggling regardless of element
visibility, matching Playwright's `_setChecked` behaviour.