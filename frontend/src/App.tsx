import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { XmlViewer } from "./components/XmlViewer";
import { GameStatePanel } from "./components/GameStatePanel";
import { LogsPanel } from "./components/LogsPanel";
import { TimelinePanel } from "./components/TimelinePanel";
import { SchedulePage } from "./components/SchedulePage";
import { QuestionsPage } from "./components/QuestionsPage";
import { ErrorReportsPanel } from "./components/ErrorReportsPanel";
import { SbgPanel } from "./components/SbgPanel";
import {
  useHealth,
  useSessions,
  useListKeys,
  useGetKey,
  useClearKey,
  getListKeysQueryKey,
} from "./api/generated";
import "./viewers.css";

const TABS = ["overview", "schedule", "questions", "timeline", "errors", "sbg"] as const;
type Tab = (typeof TABS)[number];

/** The tab named in the URL (`#schedule`, `#sbg?conn=3`), so a tab can be linked and survives a
 * reload. */
function tabOfUrl(): Tab {
  const named = window.location.hash.slice(1).split("?")[0];
  return (TABS as readonly string[]).includes(named) ? (named as Tab) : "overview";
}

export default function App() {
  const [tab, setTabState] = useState<Tab>(tabOfUrl);
  // A link to another tab, or back and forward, changes only the hash.
  useEffect(() => {
    const follow = () => setTabState(tabOfUrl());
    window.addEventListener("hashchange", follow);
    return () => window.removeEventListener("hashchange", follow);
  }, []);
  const setTab = (next: Tab) => {
    window.location.hash = next;
  };

  return (
    <div className="app">
      <header className="app-header section">
        <h1>1 vs 100 revival - admin</h1>
        <HealthBar />
      </header>
      <nav className="tab-bar section">
        <TabButton active={tab === "overview"} onClick={() => setTab("overview")}>
          Overview
        </TabButton>
        <TabButton active={tab === "schedule"} onClick={() => setTab("schedule")}>
          Schedule
        </TabButton>
        <TabButton active={tab === "questions"} onClick={() => setTab("questions")}>
          Questions
        </TabButton>
        <TabButton active={tab === "timeline"} onClick={() => setTab("timeline")}>
          Timeline
        </TabButton>
        <TabButton active={tab === "errors"} onClick={() => setTab("errors")}>
          Errors
        </TabButton>
        <TabButton active={tab === "sbg"} onClick={() => setTab("sbg")}>
          SBG
        </TabButton>
      </nav>
      {tab === "overview" && <OverviewTab />}
      {tab === "schedule" && <SchedulePage />}
      {tab === "questions" && <QuestionsPage />}
      {tab === "timeline" && <TimelineTab />}
      {tab === "errors" && (
        <div className="section">
          <ErrorReportsPanel />
        </div>
      )}
      {tab === "sbg" && (
        <div className="section">
          <SbgPanel />
        </div>
      )}
    </div>
  );
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button className={`tab-button ${active ? "tab-active" : ""}`} onClick={onClick}>
      {children}
    </button>
  );
}

function HealthBar() {
  const health = useHealth({ query: { refetchInterval: 5000 } });
  const uptime = health.data?.status === 200 ? health.data.data.uptime_secs : undefined;
  const sbgAgo = health.data?.status === 200 ? health.data.data.sbg_last_activity_secs_ago : undefined;
  const online = uptime !== undefined;
  return (
    <div className="health-bar">
      <span className={`health-dot ${online ? "" : "health-dot-off"}`} />
      {health.isLoading ? "connecting..." : online ? `server uptime ${uptime}s` : "server unreachable"}
      {online && (
        <span className="health-sub">
          | SBG activity {sbgAgo !== undefined && sbgAgo !== null ? `${sbgAgo}s ago` : "none yet"}
        </span>
      )}
    </div>
  );
}

function OverviewTab() {
  return (
    <>
      <div className="section panel-grid">
        <GameStatePanel />
        <SessionsPanel />
      </div>
      <div className="section">
        <LogsPanel />
      </div>
      <div className="section">
        <StorePanel />
      </div>
    </>
  );
}

function TimelineTab() {
  return (
    <div className="section">
      <TimelinePanel />
    </div>
  );
}

function SessionsPanel() {
  const sessions = useSessions({ query: { refetchInterval: 5000 } });
  const rows = sessions.data?.status === 200 ? sessions.data.data : [];
  return (
    <section className="card">
      <h2>Matchmaking sessions</h2>
      {rows.length === 0 && <p className="muted">No active sessions.</p>}
      {rows.map((s) => (
        <div key={s.session_id} className="session-block">
          <code>{s.session_id}</code>
          <table className="session-table">
            <thead>
              <tr>
                <th>group</th>
                <th>players</th>
                <th>last heartbeat</th>
              </tr>
            </thead>
            <tbody>
              {[...s.members]
                .sort((a, b) => a.group_id - b.group_id)
                .map((m) => (
                  <tr key={m.xuid_hex}>
                    <td>{m.group_id}</td>
                    <td>
                      {m.players.map((p) => (
                        <div key={p.xuid_hex}>
                          {p.gamertag} <code className="muted">{p.xuid_hex}</code>
                        </div>
                      ))}
                    </td>
                    <td>{m.last_heartbeat_secs_ago}s ago</td>
                  </tr>
                ))}
              {s.reservations.map((r) => (
                <tr key={`reserved-${r.xuid_hex}`} className="muted">
                  <td>{r.group_id}</td>
                  <td>
                    reserved: {r.players} player{r.players === 1 ? "" : "s"} of{" "}
                    <code>{r.xuid_hex}</code>
                  </td>
                  <td>expires in {r.expires_in_secs}s</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ))}
    </section>
  );
}

function StorePanel() {
  const queryClient = useQueryClient();
  const [prefix, setPrefix] = useState("");
  const [selected, setSelected] = useState<string | null>(null);

  const keys = useListKeys({ prefix: prefix || undefined });
  const value = useGetKey(selected ?? "", { query: { enabled: !!selected } });
  const clear = useClearKey({
    mutation: {
      onSuccess: () => {
        queryClient.invalidateQueries({ queryKey: getListKeysQueryKey({ prefix: prefix || undefined }) });
        setSelected(null);
      },
    },
  });

  const rows = keys.data?.status === 200 ? keys.data.data : [];
  const selectedValue = value.data?.status === 200 ? value.data.data : null;

  return (
    <section className="card">
      <h2>Store</h2>
      <input
        className="store-filter"
        placeholder="filter by key prefix..."
        value={prefix}
        onChange={(e) => setPrefix(e.target.value)}
      />
      <div className="panel-grid store-grid">
        <div className="store-key-list">
          <table className="store-table">
            <thead>
              <tr>
                <th>key</th>
                <th>bytes</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {rows.map((k) => (
                <tr
                  key={k.key}
                  onClick={() => setSelected(k.key)}
                  className={selected === k.key ? "row-selected" : undefined}
                >
                  <td className="store-key-cell">{k.key}</td>
                  <td>{k.len}</td>
                  <td>
                    <button
                      className="clear-button"
                      onClick={(e) => {
                        e.stopPropagation();
                        if (confirm(`Clear ${k.key}?`)) clear.mutate({ key: k.key });
                      }}
                    >
                      clear
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="store-value-pane">
          {selected ? (
            <>
              <div className="store-value-key">
                <code>{selected}</code>
              </div>
              <XmlViewer
                key={selected}
                text={
                  selectedValue?.text ??
                  selectedValue?.hex ??
                  (value.isLoading ? "loading..." : "not found")
                }
              />
            </>
          ) : (
            <p className="muted">Select a key to view its value.</p>
          )}
        </div>
      </div>
    </section>
  );
}
