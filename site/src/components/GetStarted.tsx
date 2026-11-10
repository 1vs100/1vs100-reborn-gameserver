import { DOCS_URL } from "../project";

const PLATFORMS = [
  {
    name: "Xbox 360",
    detail: "Modded console (RGH or JTAG)",
    href: `${DOCS_URL}/xbox-360/`,
  },
  {
    name: "PC",
    detail: "Windows, Linux or Mac",
    href: `${DOCS_URL}/pc/`,
  },
  {
    name: "Xbox One / Xbox Series",
    detail: "In Dev Mode",
    href: `${DOCS_URL}/xbox-one-series/`,
  },
] as const;

/** The call to action on the stage: which platforms can play, and the way to the install guides. */
export function GetStarted() {
  return (
    <section className="glass get-started" aria-labelledby="get-started-title">
      <div className="get-started-text">
        <h2 id="get-started-title" className="get-started-title">
          Join the show
        </h2>
        <p className="get-started-lead">Grab your copy of the game and pick where you want to play:</p>
        <ul className="platforms">
          {PLATFORMS.map((platform) => (
            <li key={platform.name}>
              <a className="platform" href={platform.href} rel="noopener">
                <b>{platform.name}</b>
                <span>{platform.detail}</span>
              </a>
            </li>
          ))}
        </ul>
      </div>
      <a className="cta-button" href={DOCS_URL} rel="noopener">
        How to install
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M5 12h14M13 6l6 6-6 6" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </a>
    </section>
  );
}
