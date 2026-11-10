import { useEffect, useState } from "react";
import { useGameStates } from "../api/generated";
import type { GameStateInfo, GameStatesInfo } from "../api/model";
import { ok } from "../apiResult";
import { localizeIsoTime } from "../timeFormat";
import { XmlViewer } from "./XmlViewer";

// The server reads the GameState documents (`GET /admin/gamestates`): their state, round,
// question and window, and the order a round passes through the states.

// Tracks, purely client-side, the first wall-clock time each phase was
// seen as the *current* phase - the store only ever holds the latest
// GameState, so this is the only place a "when did that happen" timestamp
// can come from. Resets whenever this page reloads.
function usePhaseLog(currentPhase: string | null) {
  const [log, setLog] = useState<Record<string, string>>({});
  useEffect(() => {
    if (!currentPhase) return;
    setLog((prev) => (prev[currentPhase] ? prev : { ...prev, [currentPhase]: new Date().toLocaleTimeString() }));
  }, [currentPhase]);
  return log;
}

function PhaseChecklist({
  phases,
  currentPhase,
  log,
}: {
  phases: string[];
  currentPhase: string | null;
  log: Record<string, string>;
}) {
  const currentIdx = currentPhase ? phases.indexOf(currentPhase) : -1;
  return (
    <div className="phase-checklist-wrap">
      <ol className="phase-checklist">
        {phases.map((phase, i) => {
          const isCurrent = i === currentIdx;
          const isDone = currentIdx !== -1 && i < currentIdx;
          return (
            <li key={phase} className={isCurrent ? "phase-current" : isDone ? "phase-done" : "phase-pending"}>
              <span
                className={`phase-marker ${isDone ? "phase-marker-done" : isCurrent ? "phase-marker-current" : "phase-marker-pending"}`}
              />
              <span className="phase-name">{phase}</span>
              {isDone && log[phase] && <span className="phase-timestamp">{log[phase]}</span>}
            </li>
          );
        })}
      </ol>
      <p className="phase-caption">as observed by this admin UI since it was opened</p>
    </div>
  );
}

export function GameStatePanel() {
  const reply = useGameStates({ query: { refetchInterval: 3000 } });
  const info = ok<GameStatesInfo>(reply.data);
  const states = info?.states ?? [];
  return (
    <section className="card game-state-panel">
      <h2>Current game state</h2>
      {reply.isLoading ? (
        <p className="muted">Loading...</p>
      ) : states.length === 0 ? (
        <p className="muted">No active game - no *.GameState key in the store.</p>
      ) : (
        <div className="game-state-grid">
          {states.map((state) => (
            <GameStateCard key={state.key} state={state} phases={info?.phases ?? []} />
          ))}
        </div>
      )}
    </section>
  );
}

function GameStateCard({ state, phases }: { state: GameStateInfo; phases: string[] }) {
  const name = state.name ?? null;
  const phaseLog = usePhaseLog(name);
  return (
    <div className="game-state-card">
      <div className="game-state-meta">
        <span className="game-state-session" title={state.key}>
          {state.session}
        </span>
      </div>
      <div className="game-state-phase">{name ?? "unreadable"}</div>
      <div className="badge-row">
        {!state.known && name && <span className="badge badge-muted">not a title state</span>}
        {state.round != null && <span className="badge">Round {state.round}</span>}
        {state.question != null && <span className="badge">Question {state.question}</span>}
        {state.end_utc && (
          <span className="badge badge-muted" title={state.end_utc}>
            ends {localizeIsoTime(state.end_utc)}
          </span>
        )}
      </div>
      <PhaseChecklist phases={phases} currentPhase={name} log={phaseLog} />
      <details className="game-state-raw">
        <summary>raw GameState value</summary>
        <XmlViewer text={state.xml} />
      </details>
    </div>
  );
}
