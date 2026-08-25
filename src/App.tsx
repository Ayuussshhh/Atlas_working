import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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

function formatGiB(bytes: number): string {
  const gib = bytes / (1024 * 1024 * 1024);
  return gib.toFixed(1) + " GB";
}

function App() {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [processInfo, setProcessInfo] = useState<ProcessInfo[] | null>(null);

  async function getSystemInfo() {
    try {
      setSystemInfo(await invoke<SystemInfo>("get_system_info"));
    } catch (error) {
      console.error("Failed to connect to the backend", error);
    }
  }

  async function getProcessInfo() {
    try {
      setProcessInfo(await invoke<ProcessInfo[]>("get_processes_info"));
    } catch (error) {
      console.error("Failed to connect ", error);
    }
  }

  return (
    <main className="container">
      <button onClick={getSystemInfo}>Get Diagnostics</button>
      {systemInfo === null ? (
        <p>There is no data</p>
      ) : (
        <>
          <p>CPU: {systemInfo.cpu_percent.toFixed(1)}%</p>
          <p>
            RAM: {formatGiB(systemInfo.used_memory)} /{" "}
            {formatGiB(systemInfo.total_memory)}
          </p>
          <p>
            DISK: {formatGiB(systemInfo.used_disk)} /{" "}
            {formatGiB(systemInfo.total_disk)}
          </p>
        </>
      )}
      <button onClick={getProcessInfo}>To Get Data</button>
      {processInfo === null ? (
        <p>No process data</p>
      ) : (
        processInfo.map((item) => (
          <p key={item.process_id}>
            {item.process_name} | PID {item.process_id} | CPU{" "}
            {item.cpu_usage.toFixed(1)}% | {formatGiB(item.memory_used)}
          </p>
        ))
      )}
    </main>
  );
}

export default App;
