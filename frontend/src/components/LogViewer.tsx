import { useEffect, useMemo, useRef, useState } from "react";
import { logLineConn, logLineTime, sbgLink } from "../sbgLink";

const LEVEL_CLASS: Record<string, string> = {
  ERROR: "log-error",
  WARN: "log-warn",
  INFO: "log-info",
  DEBUG: "log-debug",
  TRACE: "log-trace",
};

/** A record starts with a timestamp, then the level; the lines after it until the next record share its level. */
function levelLines(text: string): { line: string; className: string | undefined }[] {
  let className: string | undefined;
  return text.split("\n").filter((line, i, all) => i < all.length - 1 || line !== "").map((line) => {
    const match = /^\d\S*\s+(ERROR|WARN|INFO|DEBUG|TRACE)(?:\s|$)/.exec(line);
    if (match) className = LEVEL_CLASS[match[1]];
    return { line, className };
  });
}

export function LogViewer({ text, loading, error, filtered = false }: { text: string; loading: boolean; error: string | null; filtered?: boolean }) {
  const lines = useMemo(() => levelLines(text), [text]);
  const [autoScroll, setAutoScroll] = useState(true);
  const viewport = useRef<HTMLPreElement>(null);
  useEffect(() => {
    if (autoScroll && viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight;
  }, [text, autoScroll]);
  return <>
    <div className="log-viewer-controls">
      <label className="auto-scroll"><input type="checkbox" checked={autoScroll} onChange={(event) => setAutoScroll(event.target.checked)} /> Auto-scroll</label>
      <span className="resize-hint">drag lower-right corner to resize</span>
    </div>
    {error && <p role="alert" className="viewer-error">{error}</p>}
    <pre ref={viewport} className="log-viewer" tabIndex={0} aria-label="Log output" aria-busy={loading}>{loading
      ? "Loading log..."
      : lines.length > 0
        ? lines.map(({ line, className }, i) => <span key={i} className={className}><LogLine line={line} />{"\n"}</span>)
        : error ? "Log unavailable." : filtered ? "(no matching lines)" : "(empty)"}</pre>
  </>;
}

/** A line whose timestamp opens the SBG frames around it, of its connection when it names one. */
function LogLine({ line }: { line: string }) {
  const at = logLineTime(line);
  if (at === undefined) return <>{line}</>;
  const end = line.indexOf("Z") + 1;
  return (
    <>
      <a className="log-time" href={sbgLink(at, { conn: logLineConn(line) })} title="SBG frames around this line">
        {line.slice(0, end)}
      </a>
      {line.slice(end)}
    </>
  );
}
