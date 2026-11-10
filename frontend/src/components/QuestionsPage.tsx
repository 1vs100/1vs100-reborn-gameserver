import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  getCategoriesGetQueryKey,
  getQuestionsGetQueryKey,
  questionsExport,
  useCategoriesGet,
  useQuestionsGet,
  useQuestionsImport,
} from "../api/generated";
import type { CategoryInfo, QuestionInfo, QuestionsImportResponse } from "../api/model";
import { errorOf, ok } from "../apiResult";

// The Questions page: the categories with their counts (what a special needs),
// the served questions with their source and origin, and the bank's import and export.

export function QuestionsPage() {
  const [category, setCategory] = useState("");
  return (
    <>
      <div className="section">
        <Categories selected={category} onSelect={setCategory} />
      </div>
      <div className="section">
        <BankFile />
      </div>
      <div className="section">
        <Questions category={category} onCategory={setCategory} />
      </div>
    </>
  );
}

function Categories({ selected, onSelect }: { selected: string; onSelect: (id: string) => void }) {
  const categories = useCategoriesGet();
  const rows = ok<CategoryInfo[]>(categories.data) ?? [];
  return (
    <section className="card">
      <h2>Categories</h2>
      {rows.length === 0 && (
        <p className="muted">
          The 41 built-in questions have no category. Import a bank (below) to have categories for specials.
        </p>
      )}
      {rows.length > 0 && (
        <table className="data-table">
          <thead>
            <tr>
              <th>category</th>
              <th>sources</th>
              {[1, 2, 3, 4, 5].map((difficulty) => (
                <th key={difficulty} className="num">
                  d{difficulty}
                </th>
              ))}
              <th>special</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr
                key={row.id}
                className={`clickable ${selected === row.id ? "row-selected" : ""}`}
                onClick={() => onSelect(selected === row.id ? "" : row.id)}
              >
                <td>
                  {row.name} <code className="muted">{row.id}</code>
                </td>
                <td>{row.sources.join(", ")}</td>
                {row.counts.map((count, index) => (
                  <td key={index} className="num">
                    {count}
                  </td>
                ))}
                <td>{row.primetime ? <span className="ok-mark">enough</span> : <span className="muted">too few</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p className="legend">A special needs one Primetime round: 2, 3, 5, 4 and 7 questions at difficulties 1-5.</p>
    </section>
  );
}

function BankFile() {
  const queryClient = useQueryClient();
  const [mode, setMode] = useState<"merge" | "replace">("merge");
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const upload = useQuestionsImport({
    mutation: {
      onSuccess: (reply) => {
        const error = errorOf(reply);
        const result = ok<QuestionsImportResponse>(reply);
        setMessage({
          text: error ?? `Imported ${result?.imported} questions; ${result?.questions} are served now.`,
          error: error !== null,
        });
        if (!error) {
          queryClient.invalidateQueries({ queryKey: getCategoriesGetQueryKey() });
          queryClient.invalidateQueries({ queryKey: getQuestionsGetQueryKey() });
        }
      },
    },
  });
  const choose = async (file: File | undefined) => {
    if (!file) return;
    let body: Record<string, unknown>;
    try {
      body = JSON.parse(await file.text());
    } catch {
      setMessage({ text: `${file.name} is not JSON.`, error: true });
      return;
    }
    if (mode === "replace" && !confirm("Replace the whole imported bank with this file?")) return;
    upload.mutate({ data: body, params: { mode } });
  };
  const download = async () => {
    const reply = await questionsExport();
    const error = errorOf(reply);
    if (error) {
      setMessage({ text: error, error: true });
      return;
    }
    const url = URL.createObjectURL(new Blob([JSON.stringify(reply.data, null, 2)], { type: "application/json" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = "bank.json";
    link.click();
    URL.revokeObjectURL(url);
  };
  return (
    <section className="card">
      <h2>Question bank</h2>
      <p className="muted">
        A bank file comes from <code>question-import</code> (README). Merge replaces questions with the same id and adds
        the rest; replace drops what was imported before. It is served at once.
      </p>
      <div className="form-actions">
        <select value={mode} onChange={(e) => setMode(e.target.value as "merge" | "replace")}>
          <option value="merge">merge</option>
          <option value="replace">replace</option>
        </select>
        <label className="primary-button file-button">
          Import a bank file...
          <input
            type="file"
            accept="application/json,.json"
            onChange={(e) => {
              choose(e.target.files?.[0]);
              e.target.value = "";
            }}
            disabled={upload.isPending}
          />
        </label>
        <button className="secondary-button" onClick={download}>
          Export the imported bank
        </button>
        {upload.isPending && <span className="muted">importing...</span>}
      </div>
      {message && <p className={message.error ? "form-error" : "muted"}>{message.text}</p>}
    </section>
  );
}

function Questions({ category, onCategory }: { category: string; onCategory: (id: string) => void }) {
  const [difficulty, setDifficulty] = useState(0);
  const [limit, setLimit] = useState(100);
  const categories = ok<CategoryInfo[]>(useCategoriesGet().data) ?? [];
  const questions = useQuestionsGet({
    category: category || undefined,
    difficulty: difficulty || undefined,
    limit,
  });
  const rows = ok<QuestionInfo[]>(questions.data) ?? [];
  const credits = new Map<string, QuestionInfo>();
  for (const row of rows) if (!credits.has(row.source)) credits.set(row.source, row);
  return (
    <section className="card">
      <div className="card-header">
        <h2>Served questions</h2>
        <div className="form-actions">
          <select value={category} onChange={(e) => onCategory(e.target.value)}>
            <option value="">every category</option>
            {categories.map((known) => (
              <option key={known.id} value={known.id}>
                {known.name}
              </option>
            ))}
          </select>
          <select value={difficulty} onChange={(e) => setDifficulty(Number(e.target.value))}>
            <option value={0}>every difficulty</option>
            {[1, 2, 3, 4, 5].map((level) => (
              <option key={level} value={level}>
                difficulty {level}
              </option>
            ))}
          </select>
          <select value={limit} onChange={(e) => setLimit(Number(e.target.value))}>
            {[100, 500, 1000].map((count) => (
              <option key={count} value={count}>
                first {count}
              </option>
            ))}
          </select>
        </div>
      </div>
      {errorOf(questions.data) && <p className="form-error">{errorOf(questions.data)}</p>}
      <table className="data-table">
        <thead>
          <tr>
            <th className="num">id</th>
            <th className="num">d</th>
            <th>question</th>
            <th>choices</th>
            <th>categories</th>
            <th>source</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.id}>
              <td className="num mono">{row.id}</td>
              <td className="num">{row.difficulty}</td>
              <td>{row.text}</td>
              <td>
                {row.choices.map((choice, index) => (
                  <div key={index} className={index + 1 === row.correct ? "choice-correct" : undefined}>
                    {choice}
                  </div>
                ))}
              </td>
              <td>{row.categories.join(", ")}</td>
              <td title={row.origin_id ? `its id there: ${row.origin_id}` : undefined}>
                {row.source}
                {row.origin_category && <span className="muted"> | category {row.origin_category}</span>}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && !questions.isLoading && <p className="muted">No questions match.</p>}
      {credits.size > 0 && (
        <p className="legend">
          {[...credits.values()].map((row) => {
            const name = row.source_name ?? "unknown source";
            return (
              <span key={row.source} className="credit">
                {row.url ? <a href={row.url}>{name}</a> : name}
                {row.licence && `, ${row.licence}`}
              </span>
            );
          })}
        </p>
      )}
    </section>
  );
}
