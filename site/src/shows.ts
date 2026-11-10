import type { ShowSummary } from "./api/model";

export function showName(show: ShowSummary): string {
  switch (show.show_type) {
    case "primetime":
      return "Primetime";
    case "extended_play":
      return "Extended Play";
    case "special":
      return show.title ?? "Special episode";
  }
}

export function gameTypeText(show: ShowSummary): string {
  return show.game_type === "primetime"
    ? "The One against the Mob"
    : "Everyone plays as the Mob";
}

/** The language of a BCP 47 tag, without its region: "en-US" is "English". */
export function languageName(tag: string): string {
  const language = tag.split("-")[0];
  try {
    return new Intl.DisplayNames(["en"], { type: "language" }).of(language) ?? tag;
  } catch {
    return tag;
  }
}
