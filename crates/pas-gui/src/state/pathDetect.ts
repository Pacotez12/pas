export interface PathMatch {
  start: number;
  end: number;
  raw: string;
  path: string;
  line?: number;
  col?: number;
}

export type TextToken =
  | { kind: "text"; text: string }
  | { kind: "url"; text: string; url: string }
  | { kind: "path"; text: string; path: string; line?: number; col?: number };

const URL_RE = /https?:\/\/[^\s<>"'`)\]]+/g;

// Matches:
// 1) Quoted paths with spaces: "dir/file name.ext(:line(:col)?)?" or 'dir/file name.ext(:line(:col)?)?'
// 2) Unquoted paths containing at least one '/' (relative, ./, ../, ~/, or absolute /),
//    optionally followed by :line or :line:col.
const COMBINED_RE =
  /(https?:\/\/[^\s<>"'`)\]]+)|(["'])([^"'\r\n]*\/[^"'\r\n]+?)\2(?::(\d+)(?::(\d+))?)?|((?:(?:~\/|\.\.?\/|\/)(?:[A-Za-z0-9_@.%+-]+\/)*|(?:[A-Za-z0-9_@.%+-]+\/)+)[A-Za-z0-9_@.%+-]+(?::\d+(?::\d+)?)?)/g;

const TRAILING_PUNCT = /[.,;:!?)}\]]+$/;

function stripTrailingPunct(raw: string): string {
  return raw.replace(TRAILING_PUNCT, "");
}

function splitPathAndLocation(input: string): {
  path: string;
  line?: number;
  col?: number;
} {
  const m = input.match(/^(.*?):(\d+)(?::(\d+))?$/);
  if (!m) {
    return { path: input };
  }
  const line = Number.parseInt(m[2], 10);
  const col = m[3] ? Number.parseInt(m[3], 10) : undefined;
  return {
    path: m[1],
    line: Number.isFinite(line) && line > 0 ? line : undefined,
    col: col !== undefined && Number.isFinite(col) && col > 0 ? col : undefined,
  };
}

function isLikelyFilePath(pathPart: string): boolean {
  if (!pathPart || pathPart.length < 2) return false;
  if (pathPart.startsWith("//")) return false;
  if (/^https?:\/\//i.test(pathPart)) return false;
  if (/^[^/]+@[^/]+$/.test(pathPart)) return false;
  // Must contain a slash to distinguish from plain words/versions/timestamps.
  if (!pathPart.includes("/")) return false;
  // Reject bare "/" or "./" or "../" without a target name.
  const trimmed = pathPart.replace(/\/+$/, "");
  if (!trimmed || trimmed === "." || trimmed === "..") return false;
  return true;
}

export function findPathMatchesInLine(line: string): PathMatch[] {
  const matches: PathMatch[] = [];
  COMBINED_RE.lastIndex = 0;

  let m: RegExpExecArray | null;
  while ((m = COMBINED_RE.exec(line)) !== null) {
    // Skip URLs completely.
    if (m[1]) continue;

    if (m[3]) {
      // Quoted path.
      const inner = m[3];
      const trailingLine = m[4] ? Number.parseInt(m[4], 10) : undefined;
      const trailingCol = m[5] ? Number.parseInt(m[5], 10) : undefined;
      const parsed = splitPathAndLocation(inner);
      const finalPath = parsed.path;
      const finalLine = parsed.line ?? trailingLine;
      const finalCol = parsed.col ?? trailingCol;
      if (!isLikelyFilePath(finalPath)) continue;

      const raw = m[0];
      matches.push({
        start: m.index,
        end: m.index + raw.length,
        raw,
        path: finalPath,
        line: finalLine,
        col: finalCol,
      });
      continue;
    }

    if (m[6]) {
      const cleaned = stripTrailingPunct(m[6]);
      if (!cleaned) continue;
      // Avoid matching the path suffix of an email or something glued to a word char before `/`.
      if (m.index > 0) {
        const prev = line[m.index - 1];
        if (prev === "@" || (cleaned.startsWith("/") && /[A-Za-z0-9_-]/.test(prev))) {
          continue;
        }
      }
      const parsed = splitPathAndLocation(cleaned);
      if (!isLikelyFilePath(parsed.path)) continue;

      matches.push({
        start: m.index,
        end: m.index + cleaned.length,
        raw: cleaned,
        path: parsed.path,
        line: parsed.line,
        col: parsed.col,
      });
    }
  }

  return matches;
}

export function tokenizeText(text: string): TextToken[] {
  const tokens: TextToken[] = [];
  COMBINED_RE.lastIndex = 0;

  let cursor = 0;
  let m: RegExpExecArray | null;
  while ((m = COMBINED_RE.exec(text)) !== null) {
    if (m[1]) {
      const cleanedUrl = stripTrailingPunct(m[1]);
      if (!cleanedUrl) continue;
      if (m.index > cursor) {
        tokens.push({ kind: "text", text: text.slice(cursor, m.index) });
      }
      tokens.push({
        kind: "url",
        text: cleanedUrl,
        url: cleanedUrl,
      });
      cursor = m.index + cleanedUrl.length;
      continue;
    }

    if (m[3]) {
      const inner = m[3];
      const trailingLine = m[4] ? Number.parseInt(m[4], 10) : undefined;
      const trailingCol = m[5] ? Number.parseInt(m[5], 10) : undefined;
      const parsed = splitPathAndLocation(inner);
      const finalPath = parsed.path;
      const finalLine = parsed.line ?? trailingLine;
      const finalCol = parsed.col ?? trailingCol;
      if (!isLikelyFilePath(finalPath)) continue;

      if (m.index > cursor) {
        tokens.push({ kind: "text", text: text.slice(cursor, m.index) });
      }
      tokens.push({
        kind: "path",
        text: m[0],
        path: finalPath,
        line: finalLine,
        col: finalCol,
      });
      cursor = m.index + m[0].length;
      continue;
    }

    if (m[6]) {
      const cleaned = stripTrailingPunct(m[6]);
      if (!cleaned) continue;
      if (m.index > 0) {
        const prev = text[m.index - 1];
        if (prev === "@" || (cleaned.startsWith("/") && /[A-Za-z0-9_-]/.test(prev))) {
          continue;
        }
      }
      const parsed = splitPathAndLocation(cleaned);
      if (!isLikelyFilePath(parsed.path)) continue;

      if (m.index > cursor) {
        tokens.push({ kind: "text", text: text.slice(cursor, m.index) });
      }
      tokens.push({
        kind: "path",
        text: cleaned,
        path: parsed.path,
        line: parsed.line,
        col: parsed.col,
      });
      cursor = m.index + cleaned.length;
    }
  }

  if (cursor < text.length) {
    tokens.push({ kind: "text", text: text.slice(cursor) });
  }
  return tokens;
}

export function extractFirstDiffLine(unified: string): number | undefined {
  const m = unified.match(/^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/m);
  if (!m) return undefined;
  const line = Number.parseInt(m[1], 10);
  return Number.isFinite(line) && line > 0 ? line : undefined;
}

export function findUrlsInLine(line: string): Array<{ start: number; end: number; url: string }> {
  const results: Array<{ start: number; end: number; url: string }> = [];
  URL_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = URL_RE.exec(line)) !== null) {
    const cleaned = stripTrailingPunct(m[0]);
    if (cleaned) {
      results.push({ start: m.index, end: m.index + cleaned.length, url: cleaned });
    }
  }
  return results;
}
