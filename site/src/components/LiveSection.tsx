import { useLeaderboardPage, useLiveStatus } from "../api/generated";
import type { LeaderboardPage, LiveInfo, ShowSummary, Standings } from "../api/model";
import { ok } from "../reply";
import { gameTypeText, languageName, showName } from "../shows";
import { formatDay, formatTime, relative, useNow, zoneName } from "../time";
import { SevenSegment } from "./SevenSegment";

const POLL_MS = 5000;
/** Older data is not shown as live: the server stopped answering. */
const STALE_MS = 3 * POLL_MS;

/** The broadcast band: what is on air, the current scores and what comes next. */
export function LiveSection({ zone }: { zone: string }) {
  const reply = useLiveStatus({ query: { refetchInterval: POLL_MS } });
  const now = useNow(1000);
  const fresh = !reply.isError && now - reply.dataUpdatedAt < STALE_MS;
  const live = fresh ? ok<LiveInfo>(reply.data) : undefined;

  if (!live) {
    return (
      <div className="broadcast">
        <article className="glass panel">
          <header className="panel-head">
            <span className="live-badge off">Off air</span>
          </header>
          <div className="panel-body">
            <h2 className="show-name">
              {reply.isPending ? "Tuning in..." : "The server does not answer right now."}
            </h2>
          </div>
        </article>
      </div>
    );
  }
  return (
    <>
      <div className="broadcast">
        <OnAir live={live} zone={zone} now={now} />
        <Scores standings={live.standings ?? null} />
      </div>
      <Ticker next={live.next ?? null} zone={zone} now={now} />
    </>
  );
}

function OnAir({ live, zone, now }: { live: LiveInfo; zone: string; now: number }) {
  const show = live.current;
  const start = show ? new Date(show.start_utc).getTime() : 0;
  const end = show ? new Date(show.end_utc).getTime() : 0;
  const elapsed = end > start ? Math.min(1, Math.max(0, (now - start) / (end - start))) : 0;
  const progress = live.progress;
  return (
    <article className="glass panel on-air">
      <header className="panel-head">
        {show ? (
          <span className="live-badge">
            <span className="pulse" aria-hidden="true" /> Live
          </span>
        ) : (
          <span className="live-badge off">Off air</span>
        )}
        <span className="panel-note">
          {live.consoles} {live.consoles === 1 ? "console" : "consoles"} in the show
        </span>
      </header>
      <div className="panel-body on-air-body">
        <div>
          <h2 className="show-name">{show ? showName(show) : "Between shows"}</h2>
          <p className="show-sub">
            {show ? (
              <>
                {gameTypeText(show)}
                {show.topic && <> | {show.topic}</>} | {languageName(show.language)}
              </>
            ) : live.next ? (
              "The next show is listed below."
            ) : (
              "No show is planned."
            )}
          </p>
        </div>
        <div className="online" aria-label={`${live.players} players online`}>
          <SevenSegment value={live.players} digits={3} />
          <span className="online-label">players online</span>
        </div>
      </div>
      {show && (
        <div className="panel-body">
          <dl className="facts">
            <div>
              <dt>Now</dt>
              <dd>
                {progress ? (
                  <>
                    {progress.round != null && <>Round {progress.round}, </>}
                    question {progress.question ?? "-"}
                  </>
                ) : (
                  "Waiting for players"
                )}
              </dd>
            </div>
            <div>
              <dt>On air</dt>
              <dd>
                {formatTime(show.start_utc, zone)} - {formatTime(show.end_utc, zone)}{" "}
                <span className="muted">{zoneName(show.start_utc, zone)}</span>
              </dd>
            </div>
            <div>
              <dt>Ends</dt>
              <dd>{relative(show.end_utc, now)}</dd>
            </div>
          </dl>
          <div
            className="progress"
            role="progressbar"
            aria-label="Show time elapsed"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(elapsed * 100)}
          >
            <span style={{ width: `${elapsed * 100}%` }} />
          </div>
        </div>
      )}
    </article>
  );
}

function Scores({ standings }: { standings: Standings | null }) {
  return (
    <article className="glass panel scores">
      <header className="panel-head">
        <span className="panel-title">Top 10</span>
        <span className="panel-note">
          {standings
            ? standings.scope === "round"
              ? `Round ${standings.round}, after question ${standings.question}`
              : `This show, after question ${standings.question}`
            : "No scores yet"}
        </span>
      </header>
      {standings && standings.entries.length > 0 ? (
        <ol className="scoreboard">
          {standings.entries.map((entry) => (
            <li key={entry.gamertag}>
              <span className="sr-only">Rank {entry.rank}</span>
              <SevenSegment value={entry.rank} digits={2} className="rank" />
              <span className="gamertag">{entry.gamertag}</span>
              <span className="points">{entry.points.toLocaleString("en")}</span>
            </li>
          ))}
        </ol>
      ) : (
        <p className="panel-empty">Scores appear here once players have answered.</p>
      )}
    </article>
  );
}

function Ticker({ next, zone, now }: { next: ShowSummary | null; zone: string; now: number }) {
  const leader = useLeaderboardPage({ limit: 1 }, { query: { refetchInterval: 60000 } });
  const top = ok<LeaderboardPage>(leader.data)?.entries[0];
  return (
    <div className="ticker">
      <span className="ticker-tag">Up next</span>
      <div className="ticker-items">
        {next ? (
          <span>
            <b>{showName(next)}</b>
            {next.topic && <> ({next.topic})</>} on {formatDay(next.start_utc, zone)},{" "}
            {formatTime(next.start_utc, zone)} {zoneName(next.start_utc, zone)},{" "}
            {relative(next.start_utc, now)}
          </span>
        ) : (
          <span>No show is planned.</span>
        )}
        {top && (
          <span>
            All-time leader <b>{top.gamertag}</b> with {top.points.toLocaleString("en")} points
          </span>
        )}
      </div>
    </div>
  );
}
