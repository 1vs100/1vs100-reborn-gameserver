import { useServer } from "../api/generated";
import type { ServerInfo } from "../api/model";
import { COMMIT_URL, DISCORD_URL, GITHUB_URL } from "../project";
import { ok } from "../reply";
import { duration, formatDateTime } from "../time";
import { Wordmark } from "./Wordmark";

export function ServerFooter({ zone }: { zone: string }) {
  const reply = useServer({ query: { refetchInterval: 30000 } });
  const server = reply.isError ? undefined : ok<ServerInfo>(reply.data);
  return (
    <footer className="site-footer">
      <Wordmark />
      <p className="server-state">
        <span className={server ? "dot" : "dot down"} aria-hidden="true" />
        {server ? (
          <>
            Server online for {duration(server.uptime_secs)} | game{" "}
            {server.title_build === "1149" ? "v2.0.1149.0" : "v2.0.1317.0"} | built{" "}
            {formatDateTime(server.built_utc, zone)}
            {server.commit && (
              <>
                {" "}
                from{" "}
                <a href={COMMIT_URL + server.commit} rel="noopener">
                  {server.commit.slice(0, 7)}
                </a>
              </>
            )}
          </>
        ) : (
          "Server details are not available right now"
        )}
      </p>
      <nav className="footer-links" aria-label="Project">
        <a href={DISCORD_URL} rel="noopener">
          Discord
        </a>
        <a href={GITHUB_URL} rel="noopener">
          GitHub
        </a>
      </nav>
    </footer>
  );
}
