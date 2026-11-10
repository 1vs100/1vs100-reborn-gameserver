import { useEffect, useMemo, useState } from "react";
import { keepPreviousData, useQueryClient } from "@tanstack/react-query";
import {
  getProgrammeGetQueryKey,
  getProgrammePlanQueryKey,
  getShowsGetQueryKey,
  getSpecialsGetQueryKey,
  useCategoriesGet,
  useProgrammeGet,
  useProgrammePlan,
  useProgrammeSet,
  useShowsGet,
  useSpecialDelete,
  useSpecialPut,
  useSpecialsGet,
} from "../api/generated";
import type {
  CategoryInfo,
  PlannedShowInfo,
  ProgrammeInfo,
  ShowInfo,
  SpecialInfo,
} from "../api/model";
import { errorOf, ok } from "../apiResult";

// The Schedule page: the daily programme with a preview of what a change plans, the agenda of the
// next days, and the special episodes.

/** The longest Extended Play show: one episode; 1149 crashes joining one later. */
const EP_EPISODE_MINUTES = 30;

/** What a fresh server starts with (`Programme::default`). */
const DEFAULT_PROGRAMME: ProgrammeInfo = {
  zone: "America/New_York",
  primetime_at: ["20:00"],
  primetime_minutes: 60,
  extended_play_minutes: 30,
  min_extended_play_minutes: 30,
};

const ZONES: string[] = (() => {
  try {
    return Intl.supportedValuesOf("timeZone");
  } catch {
    return [];
  }
})();

/** The programme as `POST /admin/programme` and the plan preview take it. */
function programmeParams(programme: ProgrammeInfo) {
  return { ...programme, primetime_at: programme.primetime_at.join(",") };
}

function useRefreshSchedule() {
  const queryClient = useQueryClient();
  return () => {
    for (const queryKey of [
      getProgrammeGetQueryKey(),
      getProgrammePlanQueryKey(),
      getShowsGetQueryKey(),
      getSpecialsGetQueryKey(),
    ]) {
      queryClient.invalidateQueries({ queryKey });
    }
  };
}

export function SchedulePage() {
  const programme = useProgrammeGet();
  const saved = ok<ProgrammeInfo>(programme.data);
  const missing = programme.data?.status === 404;
  return (
    <>
      <div className="section">
        <ProgrammeCard saved={saved} missing={missing} loading={programme.isLoading} />
      </div>
      {saved && (
        <div className="section schedule-grid">
          <Agenda programme={saved} />
          <Specials programme={saved} />
        </div>
      )}
    </>
  );
}

function ProgrammeCard({
  saved,
  missing,
  loading,
}: {
  saved: ProgrammeInfo | undefined;
  missing: boolean;
  loading: boolean;
}) {
  const [draft, setDraft] = useState<ProgrammeInfo>(DEFAULT_PROGRAMME);
  const [result, setResult] = useState<string | null>(null);
  useEffect(() => {
    if (saved) setDraft(saved);
  }, [saved]);
  const changed = missing || (saved !== undefined && JSON.stringify(draft) !== JSON.stringify(saved));
  const preview = useProgrammePlan(changed ? { days: 2, ...programmeParams(draft) } : { days: 2 }, {
    query: { enabled: !loading && (changed || saved !== undefined), placeholderData: keepPreviousData },
  });
  const refresh = useRefreshSchedule();
  const save = useProgrammeSet({
    mutation: {
      onSuccess: (reply) => {
        const error = errorOf(reply);
        setResult(error ?? "Saved: the shows the Grid lists stay, the rest is planned again.");
        if (!error) refresh();
      },
    },
  });
  const field = <K extends keyof ProgrammeInfo>(key: K, value: ProgrammeInfo[K]) => {
    setResult(null);
    setDraft((current) => ({ ...current, [key]: value }));
  };
  const minutes = (key: "primetime_minutes" | "extended_play_minutes" | "min_extended_play_minutes", label: string) => (
    <label className="form-field">
      {label}
      <input
        type="number"
        min={15}
        max={key === "primetime_minutes" ? 240 : EP_EPISODE_MINUTES}
        value={draft[key]}
        onChange={(e) => field(key, Number(e.target.value))}
      />
    </label>
  );
  const previewShows = ok<PlannedShowInfo[]>(preview.data);
  const previewError = errorOf(preview.data);
  return (
    <section className="card">
      <h2>Daily programme</h2>
      {missing && (
        <p className="notice">
          No programme yet: consoles are served the unscheduled show. Check the values and start it.
        </p>
      )}
      <div className="programme-form">
        <label className="form-field form-field-wide">
          Time zone
          <input list="zones" value={draft.zone} onChange={(e) => field("zone", e.target.value)} />
          <datalist id="zones">
            {ZONES.map((zone) => (
              <option key={zone} value={zone} />
            ))}
          </datalist>
        </label>
        <fieldset className="form-field primetime-slots">
          <legend>Primetime at</legend>
          {draft.primetime_at.map((at, i) => (
            <span key={i} className="primetime-slot">
              <input
                type="time"
                value={at}
                aria-label={`Primetime start ${i + 1}`}
                onChange={(e) => field("primetime_at", draft.primetime_at.map((other, j) => (j === i ? e.target.value : other)))}
              />
              {draft.primetime_at.length > 1 && (
                <button
                  type="button"
                  className="secondary-button"
                  aria-label={`Remove Primetime start ${at}`}
                  onClick={() => field("primetime_at", draft.primetime_at.filter((_, j) => j !== i))}
                >
                  Remove
                </button>
              )}
            </span>
          ))}
          <button type="button" className="secondary-button" onClick={() => field("primetime_at", [...draft.primetime_at, ""])}>
            Add a time
          </button>
        </fieldset>
        {minutes("primetime_minutes", "Primetime (min)")}
        {minutes("extended_play_minutes", "Extended Play (min)")}
        {minutes("min_extended_play_minutes", "Shortest Extended Play (min)")}
      </div>
      <div className="form-actions">
        <button
          className="primary-button"
          disabled={!changed || save.isPending || previewError !== null}
          onClick={() => save.mutate({ params: programmeParams(draft) })}
        >
          {missing ? "Start the programme" : "Save"}
        </button>
        {changed && !missing && (
          <button className="secondary-button" onClick={() => saved && setDraft(saved)}>
            Undo changes
          </button>
        )}
        {result && <span className={errorOf(save.data) ? "form-error" : "muted"}>{result}</span>}
      </div>
      <h3 className="preview-heading">
        {changed ? "Preview: the next 48 hours with these values (not saved)" : "The next 48 hours"}
      </h3>
      {previewError && <p className="form-error">{previewError}</p>}
      {previewShows && (
        <>
          <DayRows shows={previewShows} materialised={[]} programme={draft} days={3} />
          <p className="legend">
            Primetime:{" "}
            {previewShows
              .filter((show) => show.kind !== "extended_play")
              .map((show) => `${show.start_local}${show.kind === "special" ? ` (${show.title})` : ""}`)
              .join(" | ")}
          </p>
        </>
      )}
    </section>
  );
}

function kindLabel(show: PlannedShowInfo): string {
  if (show.kind === "special") return `Special: ${show.title ?? ""} (${show.category ?? ""})`;
  return show.kind === "primetime" ? "Primetime" : "Extended Play";
}

/** `ep`, `ep-short` (the shorter show before Primetime), `pt` or `special`. */
function showClass(show: PlannedShowInfo, programme: ProgrammeInfo): string {
  if (show.kind === "special") return "special";
  if (show.kind === "primetime") return "pt";
  return show.minutes !== programme.extended_play_minutes ? "ep-short" : "ep";
}

const DAY_MINUTES = 24 * 60;

type Segment = {
  show: PlannedShowInfo;
  materialised: ShowInfo | undefined;
  /** Minutes after the day's local midnight. */
  from: number;
  length: number;
};

/** "2026-10-23" plus one day. */
function nextDay(date: string): string {
  const day = new Date(`${date}T00:00:00Z`);
  day.setUTCDate(day.getUTCDate() + 1);
  return day.toISOString().slice(0, 10);
}

/** The date and minute of the day in `zone` right now. */
function zoneNow(zone: string): { date: string; minute: number } {
  try {
    const parts = new Intl.DateTimeFormat("en-CA", {
      timeZone: zone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    }).formatToParts(new Date());
    const get = (type: string) => parts.find((part) => part.type === type)?.value ?? "00";
    return {
      date: `${get("year")}-${get("month")}-${get("day")}`,
      minute: Number(get("hour")) * 60 + Number(get("minute")),
    };
  } catch {
    return { date: new Date().toISOString().slice(0, 10), minute: 0 };
  }
}

/** Each show cut at local midnights into the days it runs in. On a daylight-saving day the
 * wall clock has 23 or 25 hours; the bar draws every day as 24, so such a day's shows shift by
 * the hour gained or lost. */
function byDay(shows: PlannedShowInfo[], materialised: ShowInfo[]): Map<string, Segment[]> {
  const days = new Map<string, Segment[]>();
  const known = new Map(materialised.map((show) => [show.start_utc, show]));
  for (const show of shows) {
    let date = show.start_local.slice(0, 10);
    let from = Number(show.start_local.slice(11, 13)) * 60 + Number(show.start_local.slice(14, 16));
    let left = show.minutes;
    while (left > 0) {
      const length = Math.min(left, DAY_MINUTES - from);
      const segments = days.get(date) ?? [];
      segments.push({ show, materialised: known.get(show.start_utc), from, length });
      days.set(date, segments);
      left -= length;
      date = nextDay(date);
      from = 0;
    }
  }
  return days;
}

function isFrozen(segment: Segment): boolean {
  const state = segment.materialised?.state;
  return state === "running" || state === "listed";
}

function Agenda({ programme }: { programme: ProgrammeInfo }) {
  const [days, setDays] = useState(7);
  const plan = useProgrammePlan({ days }, { query: { refetchInterval: 60_000 } });
  const shows = useShowsGet({ query: { refetchInterval: 60_000 } });
  const planned = ok<PlannedShowInfo[]>(plan.data) ?? [];
  const materialised = ok<ShowInfo[]>(shows.data) ?? [];
  return (
    <section className="card">
      <div className="card-header">
        <h2>Agenda ({programme.zone})</h2>
        <select value={days} onChange={(e) => setDays(Number(e.target.value))}>
          <option value={7}>7 days</option>
          <option value={14}>14 days</option>
        </select>
      </div>
      {errorOf(plan.data) && <p className="form-error">{errorOf(plan.data)}</p>}
      <DayRows shows={planned} materialised={materialised} programme={programme} days={days} />
      <p className="legend">
        <span className="legend-swatch agenda-show show-ep" /> Extended Play
        <span className="legend-swatch agenda-show show-ep-short" /> the shorter one before Primetime
        <span className="legend-swatch agenda-show show-pt" /> Primetime
        <span className="legend-swatch agenda-show show-special" /> special
        <Lock /> the Grid lists it: a change keeps it
      </p>
    </section>
  );
}

/** One bar per local day from today, `days` of them, with the shows on it and a "now" line. */
function DayRows({
  shows,
  materialised,
  programme,
  days,
}: {
  shows: PlannedShowInfo[];
  materialised: ShowInfo[];
  programme: ProgrammeInfo;
  days: number;
}) {
  const segments = useMemo(() => byDay(shows, materialised), [shows, materialised]);
  const now = zoneNow(programme.zone);
  const dates: string[] = [];
  for (let date = now.date; dates.length < days; date = nextDay(date)) dates.push(date);
  return (
    <div className="agenda">
      <div className="agenda-hours" aria-hidden>
        {[0, 6, 12, 18].map((hour) => (
          <span key={hour} style={{ left: `${(hour / 24) * 100}%` }}>
            {String(hour).padStart(2, "0")}:00
          </span>
        ))}
      </div>
      {dates.map((date) => (
        <div key={date} className="agenda-day">
          <span className="agenda-date mono">{date}</span>
          <div className="agenda-bar">
            {(segments.get(date) ?? []).map((segment) => (
              <AgendaShow key={`${segment.show.start_utc}-${segment.from}`} segment={segment} programme={programme} />
            ))}
            {date === now.date && (
              <span className="agenda-now" style={{ left: `${(now.minute / DAY_MINUTES) * 100}%` }} title="now" />
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

/** A padlock: a font-independent glyph (emoji are missing on some systems). */
function Lock() {
  return (
    <svg className="lock" viewBox="0 0 10 12" width="9" height="11" aria-label="locked">
      <rect x="1" y="5" width="8" height="6.5" rx="1" fill="currentColor" />
      <path d="M3 5V3.5a2 2 0 0 1 4 0V5" fill="none" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}

function AgendaShow({ segment, programme }: { segment: Segment; programme: ProgrammeInfo }) {
  const { show, materialised } = segment;
  const frozen = isFrozen(segment);
  const title = [
    kindLabel(show),
    `${show.start_local} (${show.minutes} min)`,
    `UTC ${show.start_utc} - ${show.end_utc}`,
    materialised && `session ${materialised.session_id}, ${materialised.state}${materialised.manual ? ", started by hand" : ""}`,
  ]
    .filter(Boolean)
    .join("\n");
  return (
    <span
      className={`agenda-show show-${showClass(show, programme)}`}
      style={{ left: `${(segment.from / DAY_MINUTES) * 100}%`, width: `${(segment.length / DAY_MINUTES) * 100}%` }}
      title={title}
    >
      {frozen && <Lock />}
      {show.kind === "special" ? show.title : show.kind === "primetime" ? "PT" : ""}
    </span>
  );
}

function Specials({ programme }: { programme: ProgrammeInfo }) {
  const specials = useSpecialsGet();
  const categories = useCategoriesGet();
  const refresh = useRefreshSchedule();
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const remove = useSpecialDelete({
    mutation: {
      onSuccess: (reply) => {
        const error = errorOf(reply);
        setMessage({ text: error ?? "Removed: that Primetime show has its regular episode back.", error: error !== null });
        if (!error) refresh();
      },
    },
  });
  const shows = useShowsGet();
  const rows = ok<SpecialInfo[]>(specials.data) ?? [];
  const known = ok<CategoryInfo[]>(categories.data) ?? [];
  // Primetime shows the Grid already lists, by local `YYYY-MM-DD HH:MM`: a special would not change them.
  const frozenStarts = new Set(
    (ok<ShowInfo[]>(shows.data) ?? [])
      .filter((show) => (show.state === "running" || show.state === "listed") && show.kind !== "extended_play")
      .map((show) => show.start_local),
  );
  return (
    <section className="card">
      <h2>Special episodes</h2>
      {rows.length === 0 && <p className="muted">No specials planned.</p>}
      {rows.length > 0 && (
        <table className="data-table">
          <thead>
            <tr>
              <th>day</th>
              <th>time</th>
              <th>title</th>
              <th>category</th>
              <th>mode</th>
              <th>artwork</th>
              <th>min</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((special) => (
              <tr key={`${special.date} ${special.at}`}>
                <td className="mono">{special.date}</td>
                <td className="mono">
                  {special.at}
                  {!programme.primetime_at.includes(special.at) && (
                    <span className="form-error"> no Primetime show at this time</span>
                  )}
                </td>
                <td>{special.title}</td>
                <td>{known.find((category) => category.id === special.category)?.name ?? special.category}</td>
                <td>{special.mode === "ep" ? "Extended Play" : "Primetime"}</td>
                <td>{special.artwork ?? ""}</td>
                <td>{special.minutes}</td>
                <td>
                  <button
                    className="clear-button"
                    onClick={() =>
                      confirm(`Remove the special on ${special.date} at ${special.at}?`) &&
                      remove.mutate({ date: special.date, at: special.at })
                    }
                  >
                    remove
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <SpecialForm programme={programme} categories={known} frozenStarts={frozenStarts} onSaved={setMessage} />
      {message && <p className={message.error ? "form-error" : "muted"}>{message.text}</p>}
    </section>
  );
}

function SpecialForm({
  programme,
  categories,
  frozenStarts,
  onSaved,
}: {
  programme: ProgrammeInfo;
  categories: CategoryInfo[];
  frozenStarts: Set<string>;
  onSaved: (message: { text: string; error: boolean }) => void;
}) {
  const [date, setDate] = useState(() => nextDay(zoneNow(programme.zone).date));
  const [chosenAt, setAt] = useState(programme.primetime_at[0]);
  // The programme's starts can change under the form.
  const at = programme.primetime_at.includes(chosenAt) ? chosenAt : programme.primetime_at[0];
  const [title, setTitle] = useState("");
  const [category, setCategory] = useState("");
  const [mode, setMode] = useState<"pt" | "ep">("pt");
  const [artwork, setArtwork] = useState("");
  const [minutes, setMinutes] = useState(programme.primetime_minutes);
  const enough = (known: CategoryInfo) => (mode === "ep" ? known.extended_play : known.primetime);
  const refresh = useRefreshSchedule();
  const put = useSpecialPut({
    mutation: {
      onSuccess: (reply) => {
        const error = errorOf(reply);
        onSaved({ text: error ?? `Saved: the Primetime show on ${date} at ${at} is the special "${title}".`, error: error !== null });
        if (!error) refresh();
      },
    },
  });
  const usable = categories.filter(enough);
  const frozen = frozenStarts.has(`${date} ${at}`);
  if (categories.length === 0) {
    return (
      <p className="notice">
        A special asks questions of one category: import a question bank first (Questions tab).
      </p>
    );
  }
  return (
    <form
      className="special-form"
      onSubmit={(e) => {
        e.preventDefault();
        put.mutate({ date, at, params: { title, category, mode, minutes, artwork: artwork || undefined } });
      }}
    >
      <h3>Make a Primetime show a special</h3>
      <label className="form-field">
        Day
        <input type="date" value={date} onChange={(e) => setDate(e.target.value)} required />
      </label>
      <label className="form-field">
        Primetime at
        <select value={at} onChange={(e) => setAt(e.target.value)}>
          {programme.primetime_at.map((start) => (
            <option key={start} value={start}>
              {start}
            </option>
          ))}
        </select>
      </label>
      <label className="form-field">
        Mode
        <select
          value={mode}
          onChange={(e) => {
            const next = e.target.value as "pt" | "ep";
            setMode(next);
            setMinutes(next === "ep" ? programme.extended_play_minutes : programme.primetime_minutes);
          }}
        >
          <option value="pt">Primetime</option>
          <option value="ep">Extended Play</option>
        </select>
      </label>
      {mode === "ep" && (
        <p className="muted">
          An Extended Play special is one episode, at most 30 minutes; regular Extended Play shows fill the rest of
          the evening.
        </p>
      )}
      <label className="form-field form-field-wide">
        Title (the theme consoles show; Extended Play only)
        <input value={title} onChange={(e) => setTitle(e.target.value)} required />
      </label>
      <label className="form-field">
        Artwork (optional)
        <input
          value={artwork}
          onChange={(e) => setArtwork(e.target.value)}
          placeholder="Community"
          pattern="[A-Za-z0-9_]+"
        />
      </label>
      <label className="form-field form-field-wide">
        Category (questions at difficulty 1-5)
        <select value={category} onChange={(e) => setCategory(e.target.value)} required>
          <option value="">choose...</option>
          {categories.map((known) => (
            <option key={known.id} value={known.id} disabled={!enough(known)}>
              {known.name} - {known.counts.join("/")}
              {enough(known) ? "" : " (too few)"}
            </option>
          ))}
        </select>
      </label>
      <label className="form-field">
        Length (min)
        <input
          type="number"
          min={15}
          max={mode === "ep" ? 30 : 240}
          value={minutes}
          onChange={(e) => setMinutes(Number(e.target.value))}
        />
      </label>
      <div className="form-actions">
        <button className="primary-button" type="submit" disabled={put.isPending || usable.length === 0 || frozen}>
          Save special
        </button>
        {usable.length === 0 && <span className="muted">No category has enough questions yet.</span>}
        {frozen && (
          <span className="form-error">
            The Grid already lists the Primetime show on {date} at {at}: it can no longer change.
          </span>
        )}
      </div>
    </form>
  );
}
