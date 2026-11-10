import { Fragment, useEffect, useState } from "react";
import { keepPreviousData, useQueryClient } from "@tanstack/react-query";
import {
  getSbgCaptureQueryKey,
  getSbgRecordingFileUrl,
  getSbgRecordingsQueryKey,
  useSbgCapture,
  useSbgEvents,
  useSbgRecordingStart,
  useSbgRecordingStop,
  useSbgRecordings,
} from "../api/generated";
import type { SbgEventsParams } from "../api/model";
import { AROUND_MS, hashParams, utcText } from "../sbgLink";

/** The filter as typed; each field is sent once it reads. */
interface Draft {
  from: string;
  to: string;
  conn: string;
  peer: string;
  xuid: string;
  key: string;
  limit: string;
}

const EMPTY: Draft = { from: "", to: "", conn: "", peer: "", xuid: "", key: "", limit: "500" };

/** The filter a `#sbg?around=...&conn=...&peer=...` link asks for. */
function draftOfUrl(): Draft {
  const params = hashParams();
  const around = Number(params.get("around"));
  const draft = { ...EMPTY, conn: params.get("conn") ?? "", peer: params.get("peer") ?? "" };
  if (params.has("around") && Number.isFinite(around)) {
    draft.from = utcText(around - AROUND_MS);
    draft.to = utcText(around + AROUND_MS);
  }
  return draft;
}

/** A typed time; one without a zone is UTC, as the server's log lines are. */
function timeOf(text: string): number | undefined {
  const trimmed = text.trim();
  if (trimmed === "") return undefined;
  const zoned = /([zZ]|[+-]\d\d:?\d\d)$/.test(trimmed) ? trimmed : `${trimmed}Z`;
  const ms = Date.parse(zoned);
  return Number.isNaN(ms) ? undefined : ms;
}

/** An address as typed, once it can be one; a partial one is not sent. */
function peerOf(text: string): string | undefined {
  const trimmed = text.trim();
  const ipv4 = /^\d{1,3}(\.\d{1,3}){3}$/.test(trimmed);
  const ipv6 = trimmed.includes(":") && /^[0-9a-fA-F:.]+$/.test(trimmed);
  return ipv4 || ipv6 ? trimmed : undefined;
}

function paramsOf(draft: Draft): SbgEventsParams {
  const conn = Number(draft.conn);
  const limit = Number(draft.limit);
  return {
    since_unix_ms: timeOf(draft.from),
    until_unix_ms: timeOf(draft.to),
    conn: draft.conn.trim() !== "" && Number.isInteger(conn) && conn >= 0 ? conn : undefined,
    peer: peerOf(draft.peer),
    xuid: /^[0-9a-fA-F]{1,16}$/.test(draft.xuid.trim()) ? draft.xuid.trim() : undefined,
    key: draft.key.trim() || undefined,
    limit: Number.isInteger(limit) && limit >= 1 ? Math.min(limit, 5000) : undefined,
  };
}

/** The SBG frames the server kept, decoded, and the recording files. */
export function SbgPanel() {
  const [draft, setDraft] = useState<Draft>(draftOfUrl);
  // The query follows the typed filter once typing pauses.
  const [params, setParams] = useState<SbgEventsParams>(() => paramsOf(draftOfUrl()));
  const [open, setOpen] = useState<number | null>(null);
  useEffect(() => {
    const timer = setTimeout(() => setParams(paramsOf(draft)), 300);
    return () => clearTimeout(timer);
  }, [draft]);
  // A link followed while the tab is open replaces the filter.
  useEffect(() => {
    const follow = () => {
      // The tab button names the tab alone and keeps the filter; a link names one.
      if (!window.location.hash.startsWith("#sbg?")) return;
      const linked = draftOfUrl();
      setDraft(linked);
      setParams(paramsOf(linked));
    };
    window.addEventListener("hashchange", follow);
    return () => window.removeEventListener("hashchange", follow);
  }, []);
  // A window that has ended does not change; only an open one is polled.
  const live = params.until_unix_ms === undefined || params.until_unix_ms > Date.now();
  const events = useSbgEvents(params, {
    query: { refetchInterval: live ? 3000 : false, placeholderData: keepPreviousData },
  });
  const rows = events.data?.status === 200 ? events.data.data : [];
  const field = (name: keyof Draft, label: string, width: string, placeholder = "") => (
    <label className="lines-input">
      {label}:{" "}
      <input
        style={{ width }}
        value={draft[name]}
        placeholder={placeholder}
        onChange={(e) => setDraft({ ...draft, [name]: e.target.value })}
      />
    </label>
  );

  return (
    <section className="card">
      <div className="timeline-header">
        <h2>SBG frames</h2>
        <button
          className="clear-button"
          onClick={() => {
            setDraft(EMPTY);
            window.history.replaceState(null, "", "#sbg");
          }}
        >
          reset filter
        </button>
      </div>
      <CaptureBar />
      <div className="sbg-filters">
        {field("from", "from", "200px", "2026-11-09T14:53:05Z")}
        {field("to", "to", "200px", "now")}
        {field("conn", "conn", "56px")}
        {field("peer", "console", "120px", "192.0.2.7")}
        {field("xuid", "xuid", "140px", "hex")}
        {field("key", "key", "160px", "GameState")}
        {field("limit", "events", "56px")}
      </div>
      {events.isError && <p className="viewer-error">Cannot load the SBG frames.</p>}
      {events.data && events.data.status !== 200 && (
        <p className="viewer-error">Cannot load the SBG frames: {events.data.data.error}</p>
      )}
      {events.data?.status === 200 && rows.length === 0 && (
        <p className="muted">No captured frames match.</p>
      )}
      {rows.length > 0 && (
        <div className="sbg-table-wrap">
        <table className="data-table sbg-table">
          <thead>
            <tr>
              <th>time (UTC)</th>
              <th>conn</th>
              <th>console</th>
              <th>dir</th>
              <th>cookie</th>
              <th>key</th>
              <th>decoded</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((event) => {
              const expanded = open === event.seq;
              return (
                <Fragment key={event.seq}>
                  <tr
                    className={`clickable sbg-${event.kind} ${expanded ? "row-selected" : ""}`}
                    onClick={() => setOpen(expanded ? null : event.seq)}
                  >
                    <td>
                      <code>{utcText(event.at_unix_ms)}</code>
                    </td>
                    <td>{event.conn}</td>
                    <td title={event.xuid_hex ?? undefined}>
                      <code>{event.peer}</code>
                    </td>
                    <td>{event.kind}</td>
                    <td>
                      {event.cookie != null && <code>{event.cookie.toString(16).padStart(4, "0")}</code>}
                    </td>
                    <td className="sbg-key">{event.key}</td>
                    <td className="sbg-text">{event.text}</td>
                  </tr>
                  {expanded && (
                    <tr>
                      <td colSpan={7}>
                        {event.xuid_hex && (
                          <div className="muted">
                            first player <code>{event.xuid_hex}</code>
                          </div>
                        )}
                        <pre className="report-document">{event.text}</pre>
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
          </tbody>
        </table>
        </div>
      )}
    </section>
  );
}

function mib(bytes: number): string {
  return (bytes / (1024 * 1024)).toFixed(1);
}

/** What the server keeps, and the recording to a file. */
function CaptureBar() {
  const queryClient = useQueryClient();
  const capture = useSbgCapture({ query: { refetchInterval: 3000 } });
  const recordings = useSbgRecordings({ query: { refetchInterval: 10000 } });
  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: getSbgCaptureQueryKey() });
    queryClient.invalidateQueries({ queryKey: getSbgRecordingsQueryKey() });
  };
  const start = useSbgRecordingStart({ mutation: { onSuccess: refresh } });
  const stop = useSbgRecordingStop({ mutation: { onSuccess: refresh } });
  const status = capture.data?.status === 200 ? capture.data.data : undefined;
  const files = recordings.data?.status === 200 ? recordings.data.data : [];
  const lastFailure = status?.failure;
  const failure =
    (start.data && start.data.status !== 200 && start.data.data.error) ||
    (stop.data && stop.data.status !== 200 && stop.data.data.error) ||
    (start.isError || stop.isError ? "the request failed" : undefined);

  return (
    <div className="sbg-capture">
      {status && (
        <p className="muted">
          {status.recent_events} frames kept ({mib(status.recent_bytes)} of{" "}
          {mib(status.recent_capacity_bytes)} MiB)
          {status.oldest_unix_ms != null && <>, since {utcText(status.oldest_unix_ms)}</>}
          {status.dropped > 0 && (
            <span className="fatal-mark">, {status.dropped} dropped because the capture fell behind</span>
          )}
        </p>
      )}
      <div className="sbg-recording">
        {status?.recording ? (
          <>
            <span>
              Recording to <code>{status.recording.name ?? status.recording.path}</code> since{" "}
              {utcText(status.recording.started_unix_ms)}, {status.recording.events} frames.
            </span>
            <button className="clear-button" disabled={stop.isPending} onClick={() => stop.mutate()}>
              stop recording
            </button>
          </>
        ) : (
          <>
            <span className="muted">Not recording to a file.</span>
            <button className="clear-button" disabled={start.isPending} onClick={() => start.mutate()}>
              start recording
            </button>
          </>
        )}
      </div>
      {failure && <p className="viewer-error">Recording: {failure}</p>}
      {!status?.recording && lastFailure && (
        <p className="viewer-error">
          The recording to <code>{lastFailure.path}</code> stopped: {lastFailure.error}
        </p>
      )}
      {files.length > 0 && (
        <p className="muted">
          Recordings for <code>sbg_replay</code>:{" "}
          {files.map((file, i) => (
            <Fragment key={file.name}>
              {i > 0 && ", "}
              <a href={getSbgRecordingFileUrl(file.name)} download={file.name}>
                {file.name}
              </a>{" "}
              ({mib(file.size_bytes)} MiB)
            </Fragment>
          ))}
        </p>
      )}
    </div>
  );
}
