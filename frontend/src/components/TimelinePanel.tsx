import { useTimeline } from "../api/generated";
import { useState } from "react";
import { localizeIsoTime } from "../timeFormat";

const CATEGORY_COLORS: Record<string, string> = {
  gamestate: "#2563eb",
  vote: "#7c3aed",
  round: "#d97706",
  presence: "#16a34a",
  mm: "#dc2626",
  matchmaking: "#dc2626",
  admin: "#64748b",
};

export function TimelinePanel() {
  const [limit, setLimit] = useState(100);
  const timeline = useTimeline({ limit }, { query: { refetchInterval: 3000 } });
  const events = timeline.data?.status === 200 ? timeline.data.data : [];

  const categories = Array.from(new Set(events.map((e) => e.category)));

  return (
    <section className="card">
      <div className="timeline-header">
        <h2>State timeline</h2>
        <label className="lines-input">
          events:{" "}
          <input
            type="number"
            value={limit}
            min={1}
            max={1000}
            onChange={(e) => setLimit(Math.max(1, Math.min(1000, Number(e.target.value) || 100)))}
          />
        </label>
      </div>
      {events.length === 0 && <p className="muted">No state changes recorded yet.</p>}
      <ul className="timeline-list">
        {events.map((ev, i) => (
          <li key={i} className="timeline-row">
            <span
              className="timeline-dot"
              style={{ background: CATEGORY_COLORS[ev.category] || "#64748b" }}
            />
            <span className="timeline-time" title={ev.timestamp}>
              {localizeIsoTime(ev.timestamp)}
            </span>
            <span className="timeline-category">{ev.category}</span>
            <span className="timeline-desc">{ev.description}</span>
            {ev.program_id && <code className="timeline-program">{ev.program_id}</code>}
          </li>
        ))}
      </ul>
      {categories.length > 0 && (
        <div className="timeline-legend">
          {categories.map((c) => (
            <span key={c} className="timeline-legend-item">
              <span
                className="timeline-dot"
                style={{ background: CATEGORY_COLORS[c] || "#64748b" }}
              />
              {c}
            </span>
          ))}
        </div>
      )}
    </section>
  );
}
