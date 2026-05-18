export type SelectionRect = {
  x: number
  y: number
  width: number
  height: number
}

export type SelectionContext = {
  selectedText: string
  filePath?: string
  lineNumber?: number
  pageUrl?: string
  frameUrl?: string
  title?: string
  selectorHint?: string
  nearestHeading?: string
  nearestLink?: string
  nearbyText?: string
  rect?: SelectionRect
  truncated?: boolean
}

export type CapturedSelectionDetails = {
  selectedText?: string
  documentText?: string
  selectorHint?: string
  nearestHeading?: string
  nearestLink?: string
  nearbyText?: string
  rect?: SelectionRect
}

export const DEFAULT_SELECTION_INTENT =
  "Use this browser selection to improve the wiki tree and related pages."

const SELECTED_TEXT_LIMIT = 4000
const NEARBY_TEXT_LIMIT = 1200
const LABEL_TEXT_LIMIT = 72

export function contextLabel(context: SelectionContext): string {
  const source = context.filePath
    ? context.lineNumber
      ? `${context.filePath}:${context.lineNumber}`
      : context.filePath
    : context.title?.trim() || context.pageUrl?.trim() || "current page"
  return `Selection: ${source} · "${previewText(context.selectedText, LABEL_TEXT_LIMIT)}"`
}

export function composePrompt(userInput: string, context: SelectionContext | null): string {
  const intent = userInput.trim() || DEFAULT_SELECTION_INTENT
  if (!context) {
    return intent
  }

  return [
    "User intent:",
    intent,
    "",
    "Browser selection context:",
    `- Title: ${context.title?.trim() || "Untitled"}`,
    `- URL: ${context.pageUrl?.trim() || "Unknown"}`,
    `- File: ${context.filePath || "Unknown"}`,
    `- Line: ${context.lineNumber?.toString() || "Unknown"}`,
    `- Element/Heading: ${contextLocation(context)}`,
    "- Selected text:",
    context.selectedText,
    "- Nearby context:",
    context.nearbyText?.trim() || "None"
  ].join("\n")
}

export function userBubbleText(userInput: string, context: SelectionContext | null): string {
  if (!context) {
    return userInput.trim()
  }
  const intent = userInput.trim() || DEFAULT_SELECTION_INTENT
  return `${intent}\n${contextLabel(context)}`
}

export function contextFromMenuClick(
  info: chrome.contextMenus.OnClickData,
  tab: chrome.tabs.Tab | undefined,
  details: CapturedSelectionDetails | null = null
): SelectionContext {
  const selected = details?.selectedText || info.selectionText || ""
  const [selectedText, selectedTruncated] = truncateText(selected, SELECTED_TEXT_LIMIT)
  const [nearbyText, nearbyTruncated] = truncateText(details?.nearbyText || "", NEARBY_TEXT_LIMIT)
  const pageUrl = info.pageUrl || tab?.url
  const filePath = workspacePathFromUrl(pageUrl)

  return {
    selectedText,
    filePath,
    lineNumber: lineNumberForSelection(details?.documentText, selectedText),
    pageUrl,
    frameUrl: info.frameUrl,
    title: tab?.title,
    selectorHint: details?.selectorHint,
    nearestHeading: details?.nearestHeading,
    nearestLink: details?.nearestLink,
    nearbyText,
    rect: details?.rect,
    truncated: selectedTruncated || nearbyTruncated || undefined
  }
}

export function captureSelectionDetails(): CapturedSelectionDetails {
  const selection = window.getSelection()
  const selectedText = selection?.toString().trim() || ""
  const range = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null
  const container = range?.commonAncestorContainer ?? null
  const element =
    container instanceof Element ? container : container?.parentElement ?? document.body
  const rect = range ? rectFromDomRect(range.getBoundingClientRect()) : undefined
  const nearestHeading = closestText(element, "h1,h2,h3,h4,h5,h6")
  const nearestLink = closestText(element, "a")
  const selectorHint = element ? selectorPath(element) : undefined
  const nearbyText = nearbyVisibleText(element)
  const documentText = document.body.innerText || document.body.textContent || ""

  return {
    selectedText,
    documentText,
    selectorHint,
    nearestHeading,
    nearestLink,
    nearbyText,
    rect
  }
}

export function workspacePathFromUrl(url: string | undefined): string | undefined {
  if (!url) {
    return undefined
  }
  try {
    const parsed = new URL(url)
    const path = decodeURIComponent(parsed.pathname)
      .replace(/^\/+/, "")
      .replace(/\/+$/, "")
    if (!path) {
      return "index.html"
    }
    if (path.endsWith(".html") || path.endsWith(".htm")) {
      return path
    }
    return `${path}/index.html`
  } catch {
    return undefined
  }
}

export function lineNumberForSelection(
  documentText: string | undefined,
  selectedText: string
): number | undefined {
  const needle = selectedText.replace(/\s+/g, " ").trim()
  if (!documentText || !needle) {
    return undefined
  }

  let offset = 0
  for (const [index, line] of documentText.split(/\r?\n/).entries()) {
    const normalizedLine = line.replace(/\s+/g, " ").trim()
    if (normalizedLine.includes(needle)) {
      return index + 1
    }
    if (needle.includes(normalizedLine) && normalizedLine.length > 0) {
      return index + 1
    }
    offset += line.length + 1
  }

  const normalizedDocument = documentText.replace(/\s+/g, " ")
  const matchOffset = normalizedDocument.indexOf(needle)
  if (matchOffset === -1) {
    return undefined
  }
  return documentText.slice(0, Math.min(offset, matchOffset)).split(/\r?\n/).length
}

function contextLocation(context: SelectionContext): string {
  return (
    context.nearestHeading?.trim() ||
    context.selectorHint?.trim() ||
    context.nearestLink?.trim() ||
    "Unknown"
  )
}

function previewText(text: string, maxChars: number): string {
  const normalized = text.replace(/\s+/g, " ").trim()
  if (normalized.length <= maxChars) {
    return normalized
  }
  return `${normalized.slice(0, maxChars - 1)}...`
}

function truncateText(text: string, maxChars: number): [string, boolean] {
  const trimmed = text.trim()
  if (trimmed.length <= maxChars) {
    return [trimmed, false]
  }
  return [`${trimmed.slice(0, maxChars)}...`, true]
}

function rectFromDomRect(rect: DOMRect): SelectionRect | undefined {
  if (rect.width === 0 && rect.height === 0) {
    return undefined
  }
  return {
    x: Math.round(rect.x),
    y: Math.round(rect.y),
    width: Math.round(rect.width),
    height: Math.round(rect.height)
  }
}

function closestText(element: Element, selector: string): string | undefined {
  const closest = element.closest(selector)
  const text = closest?.textContent?.replace(/\s+/g, " ").trim()
  return text || undefined
}

function nearbyVisibleText(element: Element | null): string | undefined {
  const text = (element?.textContent || document.body.textContent || "")
    .replace(/\s+/g, " ")
    .trim()
  return text || undefined
}

function selectorPath(element: Element): string {
  const parts: string[] = []
  let current: Element | null = element
  while (current && current !== document.body && parts.length < 5) {
    let part = current.tagName.toLowerCase()
    if (current.id) {
      part += `#${current.id}`
      parts.unshift(part)
      break
    }
    const className = Array.from(current.classList).slice(0, 2).join(".")
    if (className) {
      part += `.${className}`
    }
    parts.unshift(part)
    current = current.parentElement
  }
  return parts.join(" > ")
}
