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

type MainTab = "processes" | "library";

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
  const [clock, setClock] = useState(() => new Date());
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const [mainTab, setMainTab] = useState<MainTab>("processes");
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
    setIndexMessage(null);
    try {
      const count = await invoke<number>("index_files");
      setIndexMessage(`Indexed ${count} file${count === 1 ? "" : "s"}`);
      setMainTab("library");
      await loadDocuments();
    } catch (error) {
      console.error("Index failed", error);
      setIndexMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setIndexing(false);
    }
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

    refreshMetrics();
    loadHistory();
    loadDocuments();

    const metricsTimer = window.setInterval(refreshMetrics, 2000);
    const clockTimer = window.setInterval(() => setClock(new Date()), 1000);

    const unlistenPromise = listen<ActivityEvent>("activity-changed", (event) => {
      setActivity(event.payload);
      setHistory((prev) => [event.payload, ...prev].slice(0, 20));
    });

    return () => {
      cancelled = true;
      window.clearInterval(metricsTimer);
      window.clearInterval(clockTimer);
      unlistenPromise.then((unlisten) => unlisten());
    };
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
          <span className="mark" aria-hidden />
          <h1>Atlas</h1>
        </div>
        <div className="chrome-meta">
          <span className="status">
            <span className="status-dot" />
            Monitoring
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
                    <dt>Executable</dt>
                    <dd title={activity.path}>{activity.path || "—"}</dd>
                  </div>
                </dl>
              </div>
            ) : (
              <div className="focus-body idle">
                <p className="focus-app">No external focus</p>
                <p className="focus-title">
                  Atlas excludes itself. Focus another application to begin
                  tracking.
                </p>
              </div>
            )}
          </section>

          <section className="performance">
            <div className="section-label">Performance</div>
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
              label="Disk (C:)"
              valueLabel={
                systemInfo
                  ? `${formatGiB(systemInfo.used_disk)} / ${formatGiB(systemInfo.total_disk)}`
                  : "—"
              }
              value={diskPct}
            />
          </section>

          <section className="index-panel">
            <div className="section-label">Library</div>
            <p className="index-copy">
              Scan project text files and store metadata in SQLite.
            </p>
            <button
              type="button"
              className="btn-primary"
              onClick={handleIndex}
              disabled={indexing}
            >
              {indexing ? "Indexing…" : "Index files"}
            </button>
            {indexMessage ? <p className="index-status">{indexMessage}</p> : null}
            <p className="index-meta">{documents.length} documents in database</p>
          </section>

          <section className="timeline">
            <div className="section-label">Focus history</div>
            <ol className="timeline-list">
              {history.length === 0 ? (
                <li className="timeline-empty">Waiting for the first switch…</li>
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
                  Indexed files
                </button>
              </div>
              <p>
                {mainTab === "processes"
                  ? "Top consumers by CPU · refreshed live"
                  : "Documents upserted from the project src folder"}
              </p>
            </div>
            <span className="count">
              {mainTab === "processes"
                ? `${processInfo.length} shown`
                : `${documents.length} shown`}
            </span>
          </div>

          <div className="table-wrap">
            {mainTab === "processes" ? (
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
                        No documents yet. Use Index files in the sidebar.
                      </td>
                    </tr>
                  ) : (
                    documents.map((doc) => (
                      <tr key={doc.path}>
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
