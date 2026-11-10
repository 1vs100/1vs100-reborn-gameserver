import { useState } from "react";
import { useScheduleList } from "../api/generated";
import type { ShowSummary } from "../api/model";
import { ok } from "../reply";
import { gameTypeText, showName } from "../shows";
import { formatDay, formatTime, zoneName } from "../time";

/** Shows listed until the reader asks for all of them. */
const COLLAPSED = 5;

export function ScheduleSection({ zone }: { zone: string }) {
  const [expanded, setExpanded] = useState(false);
  const reply = useScheduleList({ query: { refetchInterval: 60000 } });
  const shows = reply.isError ? undefined : ok<ShowSummary[]>(reply.data);

  return (
    <div className="card schedule">
      <header className="card-header">
        <h3 id="schedule-table-title" className="subtitle">Next shows</h3>
        <span className="caption">Times in {zone}</span>
      </header>
      {!shows ? (
        <p className="subtle empty">
          {reply.isPending ? "Reading the schedule..." : "The schedule cannot be read right now."}
        </p>
      ) : shows.length === 0 ? (
        <p className="subtle empty">No show is planned.</p>
      ) : (
        <div className="table-scroll">
          <table className="schedule-table" aria-labelledby="schedule-table-title">
            <thead>
              <tr>
                <th scope="col">Day</th>
                <th scope="col">Time</th>
                <th scope="col">Show</th>
                <th scope="col">Game</th>
              </tr>
            </thead>
            <tbody>
              {(expanded ? shows : shows.slice(0, COLLAPSED)).map((show) => (
                <tr key={show.start_utc}>
                  <td>{formatDay(show.start_utc, zone)}</td>
                  <td className="time">
                    {formatTime(show.start_utc, zone)} - {formatTime(show.end_utc, zone)}{" "}
                    <span className="muted">{zoneName(show.start_utc, zone)}</span>
                  </td>
                  <td>
                    <b>{showName(show)}</b>
                    {show.topic && <span className="muted"> ({show.topic})</span>}
                  </td>
                  <td className="subtle">{gameTypeText(show)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {shows.length > COLLAPSED && (
            <footer className="pager">
              <span className="caption">
                {expanded ? shows.length : COLLAPSED} of {shows.length}
              </span>
              <button
                type="button"
                className="button"
                aria-expanded={expanded}
                onClick={() => setExpanded(!expanded)}
              >
                {expanded ? "Show fewer" : "Show all"}
              </button>
            </footer>
          )}
        </div>
      )}
    </div>
  );
}
