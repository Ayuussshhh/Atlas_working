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

function formatGiB(bytes: number): string {
  const gib = bytes / (1024 * 1024 * 1024);
  return gib.toFixed(1) + " GB";
}

function App() {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);

  async function getSystemInfo() {
    try {
      setSystemInfo(await invoke<SystemInfo>("get_system_info"));
    } catch (error) {
      console.error("Failed to connect to the backend", error);
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
    </main>
  );
}

export default App;
