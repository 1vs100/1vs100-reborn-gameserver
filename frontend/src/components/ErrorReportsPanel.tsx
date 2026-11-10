import { Fragment, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useErrorReports,
  useErrorReportsClear,
  getErrorReportsQueryKey,
} from "../api/generated";
import { sbgLink, utcText } from "../sbgLink";

/** The console's clock against the server's, as `+1.2 s`; the report's travel time makes it
 * look that much behind. */
function offsetText(ms: number): string {
  const sign = ms > 0 ? "+" : ms < 0 ? "-" : "";
  return `${sign}${(Math.abs(ms) / 1000).toFixed(1)} s`;
}

/** Every error report the consoles posted, newest first; the server keeps each one. */
export function ErrorReportsPanel() {
  const [limit, setLimit] = useState(200);
  // The field as typed; the limit follows it once it is a number in range.
  const [draft, setDraft] = useState("200");
  const [open, setOpen] = useState<number | null>(null);
  const queryClient = useQueryClient();
  const reports = useErrorReports({ limit }, { query: { refetchInterval: 5000 } });
  const rows = reports.data?.status === 200 ? reports.data.data : [];
  const clear = useErrorReportsClear({
    mutation: {
      onSuccess: () => {
        // Every cached list, whatever its limit.
        queryClient.invalidateQueries({ queryKey: getErrorReportsQueryKey() });
        setOpen(null);
      },
    },
  });

  return (
    <section className="card">
      <div className="timeline-header">
        <h2>Console error reports</h2>
        <label className="lines-input">
          reports:{" "}
          <input
            type="number"
            value={draft}
            min={1}
            max={5000}
            onChange={(e) => {
              setDraft(e.target.value);
              const typed = Number(e.target.value);
              if (Number.isInteger(typed) && typed >= 1 && typed <= 5000) setLimit(typed);
            }}
            onBlur={() => setDraft(String(limit))}
          />
        </label>
        <button
          className="clear-button"
          disabled={rows.length === 0 || clear.isPending}
          onClick={() => {
            if (confirm("Remove every saved error report?")) clear.mutate();
          }}
        >
          clear all
        </button>
      </div>
      {reports.isError && <p className="viewer-error">Cannot load the error reports.</p>}
      {reports.data && reports.data.status !== 200 && (
        <p className="viewer-error">Cannot load the error reports: {reports.data.data.error}</p>
      )}
      {clear.isError && <p className="viewer-error">Cannot clear the error reports.</p>}
      {clear.data && clear.data.status !== 200 && (
        <p className="viewer-error">Cannot clear the error reports: {clear.data.data.error}</p>
      )}
      {reports.data?.status === 200 && rows.length === 0 && (
        <p className="muted">No error reports saved.</p>
      )}
      {rows.length > 0 && (
        <table className="data-table">
          <thead>
            <tr>
              <th>received</th>
              <th>console</th>
              <th>name</th>
              <th>fatal</th>
              <th>state</th>
              <th>round / question</th>
              <th>reason</th>
              <th title="The console's clock less the server's when the report arrived">clock</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((report) => {
              const fields = report.fields;
              const expanded = open === report.id;
              return (
                <Fragment key={report.id}>
                  <tr
                    className={`clickable ${expanded ? "row-selected" : ""}`}
                    onClick={() => setOpen(expanded ? null : report.id)}
                  >
                    <td title={new Date(report.received_unix_ms).toISOString()}>
                      {new Date(report.received_unix_ms).toLocaleString()}
                    </td>
                    <td>
                      <code>{report.console}</code>
                    </td>
                    <td>{fields ? fields.name : <span className="muted">unknown format</span>}</td>
                    <td className={fields?.is_fatal ? "fatal-mark" : undefined}>{fields?.fatal}</td>
                    <td>{fields && `${fields.state} / ${fields.substate}`}</td>
                    <td>{fields && `${fields.round} / ${fields.question}`}</td>
                    <td>{fields?.reason}</td>
                    <td
                      title={
                        fields?.console_utc_unix_ms != null
                          ? `console clock ${utcText(fields.console_utc_unix_ms)}`
                          : undefined
                      }
                    >
                      {fields?.clock_offset_ms != null && offsetText(fields.clock_offset_ms)}
                    </td>
                  </tr>
                  {expanded && (
                    <tr>
                      <td colSpan={8}>
                        <div className="muted">
                          <code>{report.key}</code>
                          {fields && (
                            <>
                              {" "}
                              about <code>{fields.about_key}</code>
                            </>
                          )}{" "}
                          <a href={sbgLink(report.received_unix_ms, { peer: report.console })}>
                            SBG frames around this report
                          </a>
                        </div>
                        <pre className="report-document">{report.document}</pre>
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}
