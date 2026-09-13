import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

interface SearchHit {
  path: string;
  name: string;
  snippet: string;
  line_start: number | null;
  line_end: number | null;
  page_number: number | null;
  slide_number: number | null;
  file_type: string;
  location: string;
}

interface FilePreview {
  path: string;
  name: string;
  file_type: string;
  size: number;
  line_start: number | null;
  line_end: number | null;
  page_number: number | null;
  slide_number: number | null;
  location: string;
  content: string;
  query: string;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function escapeRegExp(s: string) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function highlightSnippet(snippet: string) {
  const parts = snippet.split(/(⟦|⟧)/);
  let marked = false;
  return parts.map((part, i) => {
    if (part === "⟦") {
      marked = true;
      return null;
    }
    if (part === "⟧") {
      marked = false;
      return null;
    }
    if (marked) {
      return (
        <mark key={i} className="hit-mark">
          {part}
        </mark>
      );
    }
    return <span key={i}>{part}</span>;
  });
}

function highlightPreview(content: string, query: string) {
  if (!query.trim()) return content;
  const tokens = query
    .split(/[^a-zA-Z0-9_]+/)
    .filter((t) => t.length >= 2);
  if (tokens.length === 0) return content;
  const re = new RegExp(`(${tokens.map(escapeRegExp).join("|")})`, "gi");
  const parts = content.split(re);
  return parts.map((part, i) =>
    tokens.some((t) => part.toLowerCase() === t.toLowerCase()) ? (
      <mark key={i} className="hit-mark">
        {part}
      </mark>
    ) : (
      <span key={i}>{part}</span>
    ),
  );
}

function hitToQuickPreview(hit: SearchHit, query: string): FilePreview {
  return {
    path: hit.path,
    name: hit.name,
    file_type: hit.file_type,
    size: 0,
    line_start: hit.line_start,
    line_end: hit.line_end,
    page_number: hit.page_number,
    slide_number: hit.slide_number,
    location: hit.location,
    content: hit.snippet.replace(/⟦|⟧/g, ""),
    query,
  };
}

/** Standalone Wispr-style overlay window — does not raise the main Atlas UI. */
export default function SpotlightWindow() {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [active, setActive] = useState(0);
  const [searching, setSearching] = useState(false);
  const [preview, setPreview] = useState<FilePreview | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const searchSeq = useRef(0);
  const previewSeq = useRef(0);

  async function hide() {
    try {
      await invoke("hide_spotlight");
    } catch (error) {
      console.error(error);
    }
  }

  async function runSearch(next: string) {
    const trimmed = next.trim();
    const seq = ++searchSeq.current;
    if (!trimmed) {
      setHits([]);
      setActive(0);
      setPreview(null);
      setSearching(false);
      return;
    }
    setSearching(true);
    try {
      const rows = await invoke<SearchHit[]>("search_files", { query: trimmed });
      if (seq !== searchSeq.current) return; // stale response
      setHits(rows);
      setActive(0);
    } catch (error) {
      console.error(error);
      if (seq !== searchSeq.current) return;
      setHits([]);
    } finally {
      if (seq === searchSeq.current) setSearching(false);
    }
  }

  async function loadPreview(hit: SearchHit | undefined, q: string) {
    if (!hit) {
      setPreview(null);
      return;
    }
    // Instant preview from search snippet — never block typing on disk I/O.
    setPreview(hitToQuickPreview(hit, q));
    const seq = ++previewSeq.current;
    try {
      const data = await invoke<FilePreview>("preview_file", {
        path: hit.path,
        lineStart: hit.line_start,
        lineEnd: hit.line_end,
        pageNumber: hit.page_number,
        slideNumber: hit.slide_number,
        query: q,
      });
      if (seq !== previewSeq.current) return;
      setPreview(data);
    } catch {
      // keep quick preview
    }
  }

  async function openHit(hit: SearchHit) {
    try {
      await invoke("open_path", {
        path: hit.path,
        pageNumber: hit.page_number,
      });
      await hide();
    } catch (error) {
      console.error(error);
    }
  }

  useEffect(() => {
    document.documentElement.classList.add("spotlight-root");
    document.body.classList.add("spotlight-root");
    window.setTimeout(() => inputRef.current?.focus(), 40);

    const unlisten = listen("spotlight-shown", () => {
      window.setTimeout(() => inputRef.current?.focus(), 40);
    });

    return () => {
      document.documentElement.classList.remove("spotlight-root");
      document.body.classList.remove("spotlight-root");
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const t = window.setTimeout(() => void runSearch(query), 220);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  useEffect(() => {
    const hit = hits[active];
    const t = window.setTimeout(() => void loadPreview(hit, query), 80);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hits, active]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        void hide();
        return;
      }
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActive((i) => (hits.length ? Math.min(i + 1, hits.length - 1) : 0));
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setActive((i) => Math.max(i - 1, 0));
        return;
      }
      if (event.key === "Enter" && hits[active]) {
        event.preventDefault();
        void openHit(hits[active]);
      }
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [hits, active]);

  const activeHit = hits[active];
  const isDocumentPreview =
    preview &&
    /^(pdf|docx?|pptx?|txt|md|markdown)$/i.test(preview.file_type);

  return (
    <div className="spotlight-overlay-shell">
      <div
        className="spotlight-backdrop is-window"
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) void hide();
        }}
      >
        <div className="spotlight spotlight-mac" role="dialog" aria-modal="true">
          <div className="spotlight-input-wrap">
            <svg className="spotlight-search-icon" viewBox="0 0 24 24" aria-hidden>
              <circle
                cx="11"
                cy="11"
                r="7"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
              />
              <path
                d="M20 20l-3.5-3.5"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
              />
            </svg>
            <input
              ref={inputRef}
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search inside your files…"
              autoComplete="off"
              autoFocus
            />
            <span className="spotlight-hint">{searching ? "…" : "Esc"}</span>
          </div>

          <div className="spotlight-body">
            <ul className="spotlight-results">
              {query.trim() === "" ? (
                <li className="spotlight-empty">
                  Type a phrase you remember — content search, not just filenames.
                </li>
              ) : hits.length === 0 && !searching ? (
                <li className="spotlight-empty">
                  No matches. Run Index / Update index from Atlas first.
                </li>
              ) : (
                hits.map((hit, index) => (
                  <li key={`${hit.path}-${hit.location}-${index}`}>
                    <button
                      type="button"
                      className={
                        index === active ? "spotlight-row is-active" : "spotlight-row"
                      }
                      onMouseEnter={() => setActive(index)}
                      onClick={() => void openHit(hit)}
                    >
                      <span className="spotlight-row-name">{hit.name}</span>
                      <span className="spotlight-row-meta">{hit.location}</span>
                      <span className="spotlight-row-snippet">
                        {highlightSnippet(hit.snippet)}
                      </span>
                      <span className="spotlight-row-path" title={hit.path}>
                        {hit.path}
                      </span>
                    </button>
                  </li>
                ))
              )}
            </ul>

            <aside className="spotlight-preview">
              {activeHit && preview ? (
                <div
                  className={
                    isDocumentPreview ? "file-sheet is-document" : "file-sheet"
                  }
                >
                  <div className="file-sheet-bar">
                    <span className="file-sheet-dot" />
                    <span className="file-sheet-dot" />
                    <span className="file-sheet-dot" />
                    <strong>{preview.name}</strong>
                  </div>
                  <div className="file-sheet-meta">
                    <span>{preview.location}</span>
                    <span>{preview.file_type || "file"}</span>
                    {preview.size > 0 ? (
                      <span>{formatBytes(preview.size)}</span>
                    ) : null}
                  </div>
                  <p className="file-sheet-path" title={preview.path}>
                    {preview.path}
                  </p>
                  <pre className="file-sheet-body">
                    {highlightPreview(
                      preview.content || activeHit.snippet.replace(/⟦|⟧/g, ""),
                      preview.query || query,
                    )}
                  </pre>
                  <button
                    type="button"
                    className="btn-primary preview-open"
                    onClick={() => void openHit(activeHit)}
                  >
                    Open
                    {preview.location && preview.location !== "Filename"
                      ? ` · ${preview.location}`
                      : ""}
                  </button>
                </div>
              ) : (
                <div className="preview-empty">
                  <p>Select a result to preview</p>
                </div>
              )}
            </aside>
          </div>
        </div>
      </div>
    </div>
  );
}
