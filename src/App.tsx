import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

interface SystemInfo {
  cpu_percent: number;
  used_memory: number;
  total_memory: number;
  used_disk: number;
  total_disk: number;
}

interface ProcessInfo {
  process_name: string;
  cpu_usage: number;
  memory_used: number;
  process_id: number;
}

interface ActivityEvent {
  started_at: string;
  ended_at: string;
  application: string;
  window_title: string;
  path: string;
  event_type: string;
}

interface DocumentRow {
  path: string;
  name: string;
  file_type: string;
  size: number;
  modified_at: string;
  indexed_at: string;
  hash: string;
}

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

interface IndexDoneEvent {
  ok: boolean;
  indexed: number;
  skipped: number;
  scanned: number;
  roots: string[];
  error: string | null;
}

interface IndexProgress {
  indexed: number;
  skipped: number;
  scanned: number;
  current_path: string;
}

type MainTab = "processes" | "library" | "search";

function formatGiB(bytes: number): string {
  const gib = bytes / (1024 * 1024 * 1024);
  if (gib >= 10) return gib.toFixed(1) + " GB";
  return gib.toFixed(2) + " GB";
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatTime(iso: string): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

function percent(used: number, total: number): number {
  if (!total) return 0;
  return Math.min(100, (used / total) * 100);
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

function loadTone(value: number): "ok" | "warn" | "crit" {
  if (value >= 85) return "crit";
  if (value >= 60) return "warn";
  return "ok";
}

function Meter({
  label,
  valueLabel,
  value,
}: {
  label: string;
  valueLabel: string;
  value: number;
}) {
  const clamped = Math.min(Math.max(value, 0), 100);
  return (
    <div className="meter">
      <div className="meter-top">
        <span className="meter-label">{label}</span>
        <span className={`meter-value tone-${loadTone(clamped)}`}>{valueLabel}</span>
      </div>
      <div className="meter-track" aria-hidden>
        <div
          className={`meter-fill tone-${loadTone(clamped)}`}
          style={{ width: `${clamped}%` }}
        />
      </div>
    </div>
  );
}

function App() {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [processInfo, setProcessInfo] = useState<ProcessInfo[]>([]);
  const [activity, setActivity] = useState<ActivityEvent | null>(null);
  const [history, setHistory] = useState<ActivityEvent[]>([]);
  const [documents, setDocuments] = useState<DocumentRow[]>([]);
  const [indexRoots, setIndexRoots] = useState<string[]>([]);
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [clock, setClock] = useState(() => new Date());
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const [mainTab, setMainTab] = useState<MainTab>("search");
  const [indexing, setIndexing] = useState(false);
  const [indexMessage, setIndexMessage] = useState<string | null>(null);

  async function loadDocuments() {
    try {
      const rows = await invoke<DocumentRow[]>("list_documents");
      setDocuments(rows);
    } catch (error) {
      console.error("Failed to load documents", error);
    }
  }

  async function handleIndex() {
    setIndexing(true);
    setIndexMessage(
      documents.length > 0
        ? "Checking for new or changed files…"
        : "Indexing in background… app stays usable",
    );
    try {
      await invoke("index_files");
    } catch (error) {
      console.error("Index failed", error);
      setIndexing(false);
      setIndexMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function runSearch(nextQuery: string, switchTab = true) {
    const trimmed = nextQuery.trim();
    if (!trimmed) {
      setHits([]);
      return;
    }
    setSearching(true);
    try {
      const rows = await invoke<SearchHit[]>("search_files", { query: trimmed });
      setHits(rows);
      if (switchTab) setMainTab("search");
    } catch (error) {
      console.error("Search failed", error);
      setHits([]);
    } finally {
      setSearching(false);
    }
  }

  async function handleOpen(path: string, pageNumber?: number | null) {
    try {
      await invoke("open_path", {
        path,
        pageNumber: pageNumber ?? null,
      });
    } catch (error) {
      console.error("Open failed", error);
    }
  }

  function openSpotlightOverlay() {
    void invoke("open_spotlight_window");
  }

  useEffect(() => {
    let cancelled = false;

    async function refreshMetrics() {
      try {
        const [sys, procs] = await Promise.all([
          invoke<SystemInfo>("get_system_info"),
          invoke<ProcessInfo[]>("get_processes_info"),
        ]);
        if (!cancelled) {
          setSystemInfo(sys);
          setProcessInfo(procs);
        }
      } catch (error) {
        console.error("Failed to refresh metrics", error);
      }
    }

    async function loadHistory() {
      try {
        const rows = await invoke<ActivityEvent[]>("get_recent_activity");
        if (!cancelled) {
          setHistory(rows);
          setActivity((prev) => prev ?? rows[0] ?? null);
        }
      } catch (error) {
        console.error("Failed to load activity", error);
      }
    }

    async function loadRoots() {
      try {
        const roots = await invoke<string[]>("get_index_roots");
        if (!cancelled) setIndexRoots(roots);
      } catch (error) {
        console.error("Failed to load index roots", error);
      }
    }

    refreshMetrics();
    loadHistory();
    loadDocuments();
    loadRoots();

    const metricsTimer = window.setInterval(refreshMetrics, 2000);
    const clockTimer = window.setInterval(() => setClock(new Date()), 1000);

    const unlistenPromise = listen<ActivityEvent>("activity-changed", (event) => {
      setActivity(event.payload);
      setHistory((prev) => [event.payload, ...prev].slice(0, 20));
    });

    const unlistenProgress = listen<IndexProgress>("index-progress", (event) => {
      setIndexing(true);
      const { indexed, skipped, scanned, current_path } = event.payload;
      setIndexMessage(
        `Updating… ${indexed} new · ${skipped} unchanged · ${scanned} scanned · ${current_path}`,
      );
    });

    const unlistenDone = listen<IndexDoneEvent>("index-done", async (event) => {
      setIndexing(false);
      if (event.payload.ok) {
        setIndexRoots(event.payload.roots);
        const { indexed, skipped, scanned } = event.payload;
        if (indexed === 0 && skipped > 0) {
          setIndexMessage(
            `Up to date — ${skipped} unchanged file${skipped === 1 ? "" : "s"} (scanned ${scanned})`,
          );
        } else {
          setIndexMessage(
            `${indexed} new/changed · ${skipped} unchanged · ${scanned} scanned (pdf/docx/pptx included)`,
          );
        }
        await loadDocuments();
      } else {
        setIndexMessage(event.payload.error ?? "Index failed");
      }
    });

    return () => {
      cancelled = true;
      window.clearInterval(metricsTimer);
      window.clearInterval(clockTimer);
      unlistenPromise.then((unlisten) => unlisten());
      unlistenProgress.then((unlisten) => unlisten());
      unlistenDone.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const handle = window.setTimeout(() => {
      void runSearch(query, false);
    }, 280);
    return () => window.clearTimeout(handle);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const isMod = event.ctrlKey || event.metaKey;
      const kCombo = isMod && event.key.toLowerCase() === "k";
      const shiftSpace =
        isMod && event.shiftKey && event.code === "Space";
      if (kCombo || shiftSpace) {
        event.preventDefault();
        openSpotlightOverlay();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const cpu = systemInfo?.cpu_percent ?? 0;
  const ramPct = systemInfo
    ? percent(systemInfo.used_memory, systemInfo.total_memory)
    : 0;
  const diskPct = systemInfo
    ? percent(systemInfo.used_disk, systemInfo.total_disk)
    : 0;

  return (
    <div className="app">
      <header className="chrome">
        <div className="chrome-brand">
          <img
            className="brand-mark"
            src="/book.png"
            alt=""
            width={28}
            height={28}
          />
          <div>
            <h1>Atlas</h1>
            <p className="chrome-tag">Observe · Remember · Find · Navigate</p>
          </div>
        </div>
        <div className="chrome-meta">
          <button
            type="button"
            className="btn-ghost"
            onClick={openSpotlightOverlay}
          >
            Spotlight
            <kbd>Ctrl</kbd>
            <kbd>Win</kbd>
            <kbd>Space</kbd>
          </button>
          <span className="status">
            <span className="status-dot" />
            Live
          </span>
          <time dateTime={clock.toISOString()}>
            {clock.toLocaleTimeString([], { hour12: false })}
          </time>
        </div>
      </header>

      <div className="workspace">
        <aside className="rail">
          <section className="focus">
            <div className="section-label">Active window</div>
            {activity ? (
              <div key={activity.started_at} className="focus-body">
                <p className="focus-app">{activity.application}</p>
                <p className="focus-title">
                  {activity.window_title || "Untitled window"}
                </p>
                <dl className="focus-facts">
                  <div>
                    <dt>Started</dt>
                    <dd>{formatTime(activity.started_at)}</dd>
                  </div>
                  <div>
                    <dt>Path</dt>
                    <dd title={activity.path}>{activity.path || "—"}</dd>
                  </div>
                </dl>
              </div>
            ) : (
              <div className="focus-body idle">
                <p className="focus-app">Waiting for focus</p>
                <p className="focus-title">
                  Switch to another app — Atlas tracks the foreground window.
                </p>
              </div>
            )}
          </section>

          <section className="performance">
            <div className="section-label">Machine</div>
            <Meter label="CPU" valueLabel={`${cpu.toFixed(1)}%`} value={cpu} />
            <Meter
              label="Memory"
              valueLabel={
                systemInfo
                  ? `${formatGiB(systemInfo.used_memory)} / ${formatGiB(systemInfo.total_memory)}`
                  : "—"
              }
              value={ramPct}
            />
            <Meter
              label="Disk C:"
              valueLabel={
                systemInfo
                  ? `${formatGiB(systemInfo.used_disk)} / ${formatGiB(systemInfo.total_disk)}`
                  : "—"
              }
              value={diskPct}
            />
          </section>

          <section className="index-panel">
            <div className="section-label">Remember</div>
            <p className="index-copy">
              Scans Documents, Desktop, Downloads (and OneDrive). Indexes PDF,
              DOCX, PPTX, and code. Skips deep project folders. After the first
              run, only new or changed files are processed. Image-only / scanned
              files stay findable by name.
            </p>
            <button
              type="button"
              className="btn-primary"
              onClick={handleIndex}
              disabled={indexing}
            >
              {indexing
                ? "Updating in background…"
                : documents.length > 0
                  ? "Update index"
                  : "Index this PC"}
            </button>
            {indexMessage ? <p className="index-status">{indexMessage}</p> : null}
            <p className="index-meta">{documents.length} documents remembered</p>
            {indexRoots.length > 0 ? (
              <ul className="root-list">
                {indexRoots.map((root) => (
                  <li key={root} title={root}>
                    {root}
                  </li>
                ))}
              </ul>
            ) : null}
          </section>

          <section className="timeline">
            <div className="section-label">Focus history</div>
            <ol className="timeline-list">
              {history.length === 0 ? (
                <li className="timeline-empty">No switches yet…</li>
              ) : (
                history.map((item, index) => {
                  const isCurrent =
                    index === 0 && activity?.started_at === item.started_at;
                  return (
                    <li
                      key={`${item.started_at}-${item.application}-${index}`}
                      className={isCurrent ? "is-current" : undefined}
                    >
                      <time>{formatTime(item.started_at)}</time>
                      <div>
                        <strong>{item.application}</strong>
                        <span>{item.window_title || "Untitled"}</span>
                      </div>
                    </li>
                  );
                })
              )}
            </ol>
          </section>
        </aside>

        <main className="main">
          <div className="table-toolbar">
            <div>
              <div className="tab-row" role="tablist">
                <button
                  type="button"
                  role="tab"
                  aria-selected={mainTab === "search"}
                  className={mainTab === "search" ? "tab is-active" : "tab"}
                  onClick={() => setMainTab("search")}
                >
                  Find
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={mainTab === "processes"}
                  className={mainTab === "processes" ? "tab is-active" : "tab"}
                  onClick={() => setMainTab("processes")}
                >
                  Processes
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={mainTab === "library"}
                  className={mainTab === "library" ? "tab is-active" : "tab"}
                  onClick={() => setMainTab("library")}
                >
                  Library
                </button>
              </div>
              <p>
                {mainTab === "search"
                  ? "Global Spotlight: Ctrl+Win+Space (works while Atlas is in the background)"
                  : mainTab === "processes"
                    ? "Top CPU consumers · live sample"
                    : "Indexed file catalog · click to open"}
              </p>
            </div>
            <span className="count">
              {mainTab === "search"
                ? searching
                  ? "Searching…"
                  : `${hits.length} hits`
                : mainTab === "processes"
                  ? `${processInfo.length} shown`
                  : `${documents.length} shown`}
            </span>
          </div>

          {mainTab === "search" ? (
            <div className="search-bar">
              <input
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search across indexed files on this PC…"
              />
            </div>
          ) : null}

          <div className="table-wrap">
            {mainTab === "search" ? (
              <ul className="hit-list">
                {query.trim() === "" ? (
                  <li className="hit-empty">
                    <div className="empty-state">
                      <h3>Search your machine</h3>
                      <p>
                        Click <strong>Update index</strong> (or Index this PC
                        once), then press{" "}
                        <strong>Ctrl+Win+Space</strong> anytime — even when
                        Atlas is minimized — for Spotlight with file preview.
                      </p>
                    </div>
                  </li>
                ) : hits.length === 0 && !searching ? (
                  <li className="hit-empty">
                    <div className="empty-state">
                      <h3>No matches</h3>
                      <p>Try another word, or re-index after adding files.</p>
                    </div>
                  </li>
                ) : (
                  hits.map((hit, index) => (
                    <li key={`${hit.path}-${hit.location}-${index}`}>
                      <button
                        type="button"
                        className="hit"
                        onClick={() => handleOpen(hit.path, hit.page_number)}
                      >
                        <div className="hit-top">
                          <strong>{hit.name}</strong>
                          <span className="hit-lines">{hit.location}</span>
                        </div>
                        <p className="hit-snippet">{highlightSnippet(hit.snippet)}</p>
                        <p className="hit-path" title={hit.path}>
                          {hit.path}
                        </p>
                      </button>
                    </li>
                  ))
                )}
              </ul>
            ) : mainTab === "processes" ? (
              <table>
                <thead>
                  <tr>
                    <th className="col-name">Name</th>
                    <th className="col-pid">PID</th>
                    <th className="col-cpu">CPU</th>
                    <th className="col-mem">Memory</th>
                    <th className="col-bar" aria-label="CPU share" />
                  </tr>
                </thead>
                <tbody>
                  {processInfo.length === 0 ? (
                    <tr>
                      <td colSpan={5} className="table-empty">
                        Collecting process samples…
                      </td>
                    </tr>
                  ) : (
                    processInfo.map((item) => {
                      const share = Math.min(item.cpu_usage, 100);
                      return (
                        <tr
                          key={item.process_id}
                          className={
                            selectedPid === item.process_id
                              ? "is-selected"
                              : undefined
                          }
                          onClick={() => setSelectedPid(item.process_id)}
                        >
                          <td className="col-name">{item.process_name}</td>
                          <td className="col-pid">{item.process_id}</td>
                          <td className={`col-cpu tone-${loadTone(share)}`}>
                            {item.cpu_usage.toFixed(1)}%
                          </td>
                          <td className="col-mem">
                            {formatGiB(item.memory_used)}
                          </td>
                          <td className="col-bar">
                            <div className="spark">
                              <div style={{ width: `${share}%` }} />
                            </div>
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            ) : (
              <table>
                <thead>
                  <tr>
                    <th className="col-doc-name">Name</th>
                    <th className="col-doc-type">Type</th>
                    <th className="col-doc-size">Size</th>
                    <th className="col-doc-path">Path</th>
                    <th className="col-doc-time">Indexed</th>
                  </tr>
                </thead>
                <tbody>
                  {documents.length === 0 ? (
                    <tr>
                      <td colSpan={5} className="table-empty">
                        No documents yet. Click Index this PC once.
                      </td>
                    </tr>
                  ) : (
                    documents.map((doc) => (
                      <tr
                        key={doc.path}
                        className="is-clickable"
                        onClick={() => handleOpen(doc.path)}
                      >
                        <td className="col-doc-name" title={doc.name}>
                          {doc.name}
                        </td>
                        <td className="col-doc-type">{doc.file_type || "—"}</td>
                        <td className="col-doc-size">{formatBytes(doc.size)}</td>
                        <td className="col-doc-path" title={doc.path}>
                          {doc.path}
                        </td>
                        <td className="col-doc-time">
                          {formatTime(doc.indexed_at)}
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            )}
          </div>
        </main>
      </div>
    </div>
  );
}

export default App;
