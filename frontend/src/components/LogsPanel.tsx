import { useEffect, useState } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { getLogTailUrl, useLogs } from "../api/generated";
import type { LogTailParams } from "../api/model";
import { LogViewer } from "./LogViewer";

type Level = NonNullable<LogTailParams["level"]>;
const LEVELS: Level[] = ["error", "warn", "info", "debug", "trace"];

function useLogTail(name: string, params: LogTailParams) {
  const url = getLogTailUrl(name, params);
  return useQuery({
    queryKey: [url],
    queryFn: async ({ signal }) => {
      // This endpoint returns plain text; the generated client assumes JSON.
      const response = await fetch(url, { signal });
      if (!response.ok) throw new Error(`Unable to load log (HTTP ${response.status}).`);
      return response.text();
    },
    refetchInterval: 5000,
    // A changed filter keeps the old lines on screen until the new ones arrive.
    placeholderData: keepPreviousData,
  });
}

function LogTail({ title, name, defaultLines = 100 }: { title: string; name: string; defaultLines?: number }) {
  const [lines, setLines] = useState(defaultLines);
  const [level, setLevel] = useState<Level | "">("");
  const [contains, setContains] = useState("");
  // The filter as typed; the query follows once typing pauses.
  const [query, setQuery] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setQuery(contains), 300);
    return () => clearTimeout(timer);
  }, [contains]);
  const tail = useLogTail(name, {
    lines,
    level: level || undefined,
    contains: query || undefined,
  });
  return (
    <div className="log-tail">
      <div className="log-tail-header">
        <h3>{title}</h3>
        <label className="lines-input">
          level:{" "}
          <select value={level} onChange={(e) => setLevel(e.target.value as Level | "")}>
            <option value="">all</option>
            {LEVELS.map((l) => (
              <option key={l} value={l}>
                {l} and above
              </option>
            ))}
          </select>
        </label>
        <label className="lines-input">
          filter:{" "}
          <input
            type="search"
            className="log-filter-input"
            value={contains}
            placeholder="text"
            onChange={(e) => setContains(e.target.value)}
          />
        </label>
        <label className="lines-input">
          lines:{" "}
          <input
            type="number"
            value={lines}
            min={1}
            max={5000}
            onChange={(e) => setLines(Math.max(1, Math.min(5000, Number(e.target.value) || defaultLines)))}
          />
        </label>
      </div>
      <LogViewer text={tail.data ?? ""} filtered={level !== "" || query !== ""} loading={tail.isLoading} error={tail.error?.message ?? null} />
    </div>
  );
}

function prettyTitle(name: string): string {
  return name.replace(/\.log$/, "").replace(/_/g, " ");
}

export function LogsPanel() {
  // Whatever this session's log directory holds - new log files appear
  // here with no frontend change.
  const logs = useLogs({ query: { refetchInterval: 10000 } });
  const names = logs.data?.status === 200 ? logs.data.data.map((f) => f.name) : [];
  return (
    <section className="card">
      <h2>Logs</h2>
      {logs.isLoading ? (
        <p className="muted">Loading...</p>
      ) : names.length === 0 ? (
        <p className="muted">No log files yet.</p>
      ) : (
        <div className="logs-grid">
          {names.map((name) => (
            <LogTail key={name} title={prettyTitle(name)} name={name} />
          ))}
        </div>
      )}
    </section>
  );
}
