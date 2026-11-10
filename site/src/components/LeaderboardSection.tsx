import { useState } from "react";
import { keepPreviousData } from "@tanstack/react-query";
import { useLeaderboardPage } from "../api/generated";
import type { LeaderboardPage } from "../api/model";
import { ok } from "../reply";

const PAGE = 20;

export function LeaderboardSection() {
  const [offset, setOffset] = useState(0);
  const reply = useLeaderboardPage(
    { offset, limit: PAGE },
    { query: { refetchInterval: 60000, placeholderData: keepPreviousData } },
  );
  const page = reply.isError ? undefined : ok<LeaderboardPage>(reply.data);
  const last = page ? Math.min(page.offset + PAGE, page.total) : 0;

  return (
    <div className="card leaderboard">
      <header className="card-header">
        <h3 className="subtitle">All-time leaderboard</h3>
        <span className="caption">By lifetime score</span>
      </header>
      {!page ? (
        <p className="subtle empty">
          {reply.isPending ? "Reading the leaderboard..." : "The leaderboard cannot be read right now."}
        </p>
      ) : page.total === 0 ? (
        <p className="subtle empty">No public players yet.</p>
      ) : (
        <>
          <ol className="ranking wide" start={page.offset + 1}>
            {page.entries.map((entry) => (
              <li key={entry.gamertag} className={entry.rank <= 3 ? `top top-${entry.rank}` : undefined}>
                <span className="rank">{entry.rank}</span>
                <span className="gamertag">{entry.gamertag}</span>
                <span className="points">{entry.points.toLocaleString("en")}</span>
              </li>
            ))}
          </ol>
          <footer className="pager">
            <span className="caption">
              {page.offset + 1} - {last} of {page.total.toLocaleString("en")}
            </span>
            <div className="pager-buttons">
              <button
                type="button"
                className="button"
                disabled={offset === 0}
                onClick={() => setOffset(Math.max(0, offset - PAGE))}
              >
                Previous
              </button>
              <button
                type="button"
                className="button"
                disabled={last >= page.total}
                onClick={() => setOffset(offset + PAGE)}
              >
                Next
              </button>
            </div>
          </footer>
        </>
      )}
      <p className="note">
        Only players who chose <strong>Public</strong> in the game's privacy settings are listed. To
        appear here, or to leave, change it under Options, Privacy in 1 vs 100.
      </p>
    </div>
  );
}
